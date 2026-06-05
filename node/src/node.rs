use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use veilux_kernel::{Block, Cascade, Command, Hash, PartyId, SignedCommand, StateTree};
use veilux_store::Store;
use veilux_veil::{project_block, verify_signed, SubLedger, ViewKeyring};

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error(transparent)]
    Cascade(#[from] veilux_kernel::CascadeError),
    #[error(transparent)]
    View(#[from] veilux_veil::ViewError),
    #[error("mempool is empty")]
    EmptyMempool,
    #[error("signature verification failed: {0}")]
    BadSignature(String),
    #[error("unknown prism '{0}'")]
    UnknownPrism(String),
    #[error("replayed or out-of-order nonce for {party}: got {got}, expected >= {expected}")]
    BadNonce {
        party: String,
        got: u64,
        expected: u64,
    },
    #[error("party {0} is bound to a different signing key")]
    KeyMismatch(String),
    #[error("'{0}' is a reserved/system account and cannot submit commands")]
    ReservedAccount(String),
    #[error("command too large: {0} bytes")]
    TooLarge(usize),
    #[error("mempool is full")]
    MempoolFull,
    #[error("store error: {0}")]
    Store(String),
    #[error("block parent/height does not extend the current head")]
    BadParent,
    #[error("block timestamp {got} is not >= parent {parent} (or too far in the future)")]
    BadTimestamp { got: u64, parent: u64 },
    #[error("block exceeds gas limit: used {used}, limit {limit}")]
    BlockGasExceeded { used: u64, limit: u64 },
    #[error("{0} mismatch: block does not match re-executed result")]
    RootMismatch(String),
}

pub struct Limits {
    pub max_payload_bytes: usize,
    pub max_block_commands: usize,
    pub max_block_gas: u64,
    pub max_mempool: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_payload_bytes: 1024 * 1024,
            max_block_commands: 10_000,
            max_block_gas: 30_000_000,
            max_mempool: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FeePolicy {
    pub price_per_gas: u128,
    pub burn_bps: u16,
    pub target_gas: u64,
}

impl Default for FeePolicy {
    fn default() -> Self {
        FeePolicy {
            price_per_gas: 0,
            burn_bps: 5_000,
            target_gas: 0,
        }
    }
}

impl FeePolicy {
    pub fn enabled(&self) -> bool {
        self.price_per_gas > 0
    }

    pub fn dynamic(&self) -> bool {
        self.enabled() && self.target_gas > 0
    }
}

const BASE_PRICE_KEY: &str = "fee/base_price";
const MAX_FUTURE_DRIFT_SECS: u64 = 7_200;

pub struct Node {
    pub cascade: Cascade,
    pub state: StateTree,
    pub blocks: Vec<Block>,
    pub mempool: Vec<Command>,
    pub keyrings: Vec<ViewKeyring>,
    pub sub_ledgers: HashMap<PartyId, SubLedger>,
    pub proposer: PartyId,
    pub accounts: HashMap<PartyId, Vec<u8>>,
    pub nonces: HashMap<PartyId, u64>,
    pub limits: Limits,
    pub fee_policy: FeePolicy,
    pub store: Option<Store>,
}

impl Node {
    pub fn new(proposer: PartyId, cascade: Cascade) -> Self {
        let genesis = Block::deterministic_genesis();
        Node {
            cascade,
            state: StateTree::new(),
            blocks: vec![genesis],
            mempool: Vec::new(),
            keyrings: Vec::new(),
            sub_ledgers: HashMap::new(),
            proposer,
            accounts: HashMap::new(),
            nonces: HashMap::new(),
            limits: Limits::default(),
            fee_policy: FeePolicy::default(),
            store: None,
        }
    }

    pub fn with_store(
        proposer: PartyId,
        cascade: Cascade,
        store: Store,
    ) -> Result<Self, NodeError> {
        let mut node = Node::new(proposer, cascade);
        let existing = store
            .load_blocks()
            .map_err(|e| NodeError::Store(e.to_string()))?;
        if !existing.is_empty() {
            node.blocks = existing;
            if let Some(state) = store
                .load_state()
                .map_err(|e| NodeError::Store(e.to_string()))?
            {
                node.state = state;
            }
        } else {
            store
                .append_block(&node.blocks[0])
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }
        node.store = Some(store);
        Ok(node)
    }

    pub fn host_party(&mut self, keyring: ViewKeyring) {
        let party = keyring.party().clone();
        self.sub_ledgers
            .entry(party.clone())
            .or_insert_with(|| SubLedger::new(party));
        self.keyrings.push(keyring);
    }

    pub fn is_fresh(&self) -> bool {
        self.blocks.len() == 1 && self.state.is_empty()
    }

    pub fn seed_genesis_state<F>(&mut self, seeder: F) -> Result<bool, NodeError>
    where
        F: FnOnce(&mut StateTree) -> Result<(), NodeError>,
    {
        if !self.is_fresh() {
            return Ok(false);
        }
        seeder(&mut self.state)?;
        if let Some(store) = &self.store {
            store
                .save_state(&self.state)
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }
        Ok(true)
    }

    fn base_price(state: &StateTree, policy: FeePolicy) -> u128 {
        state
            .get_json::<String>(BASE_PRICE_KEY)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(policy.price_per_gas)
    }

    fn charge_fee(
        policy: FeePolicy,
        price: u128,
        state: &mut StateTree,
        payer: &PartyId,
        proposer: &PartyId,
        gas_used: u64,
    ) {
        if !policy.enabled() || price == 0 {
            return;
        }
        let fee = price.saturating_mul(gas_used as u128);
        if fee == 0 {
            return;
        }
        let _ = prism_token::collect_fee(
            state,
            &prism_token::native_token_id(),
            payer,
            proposer,
            fee,
            policy.burn_bps,
        );
    }

    fn adjust_base_price(state: &mut StateTree, policy: FeePolicy, block_gas: u64) {
        if !policy.dynamic() {
            return;
        }
        let current = Self::base_price(state, policy);
        let target = policy.target_gas as u128;
        let used = block_gas as u128;
        let delta = current.max(1) / 8;
        let next = match used.cmp(&target) {
            std::cmp::Ordering::Greater => {
                let scaled = delta.saturating_mul(used - target) / target;
                current.saturating_add(scaled.max(1))
            }
            std::cmp::Ordering::Less => {
                let scaled = delta.saturating_mul(target - used) / target;
                current
                    .saturating_sub(scaled)
                    .max(policy.price_per_gas.max(1))
            }
            std::cmp::Ordering::Equal => current,
        };
        let _ = state.put_json(BASE_PRICE_KEY, &next.to_string());
    }

    pub fn head(&self) -> &Block {
        self.blocks.last().expect("chain always has genesis")
    }

    pub fn blocks_from(&self, from_height: u64, max: usize) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|b| b.height >= from_height && b.height > 0)
            .take(max)
            .cloned()
            .collect()
    }

    pub fn estimate(&self, command: &Command) -> Result<u64, NodeError> {
        Ok(self.cascade.estimate(command, &self.state)?)
    }

    pub fn submit_signed(&mut self, signed: SignedCommand) -> Result<(), NodeError> {
        let cmd = &signed.command;

        if cmd.payload.len() > self.limits.max_payload_bytes {
            return Err(NodeError::TooLarge(cmd.payload.len()));
        }

        if cmd.submitter.0.contains('/') || cmd.submitter.0.is_empty() {
            return Err(NodeError::ReservedAccount(cmd.submitter.0.clone()));
        }

        verify_signed(&signed).map_err(|e| NodeError::BadSignature(e.to_string()))?;

        match self.accounts.get(&cmd.submitter) {
            Some(existing) if existing != &signed.public_key => {
                return Err(NodeError::KeyMismatch(cmd.submitter.0.clone()));
            }
            None => {
                self.accounts
                    .insert(cmd.submitter.clone(), signed.public_key.clone());
            }
            _ => {}
        }

        if !self.cascade.has(&cmd.prism) {
            return Err(NodeError::UnknownPrism(cmd.prism.clone()));
        }

        let expected = self.nonces.get(&cmd.submitter).map(|n| n + 1).unwrap_or(0);
        if cmd.nonce < expected {
            return Err(NodeError::BadNonce {
                party: cmd.submitter.0.clone(),
                got: cmd.nonce,
                expected,
            });
        }

        if self.mempool.len() >= self.limits.max_mempool {
            return Err(NodeError::MempoolFull);
        }

        self.nonces.insert(cmd.submitter.clone(), cmd.nonce);
        self.mempool.push(signed.command);
        Ok(())
    }

    pub fn produce_block(&mut self) -> Result<BlockSummary, NodeError> {
        if self.mempool.is_empty() {
            return Err(NodeError::EmptyMempool);
        }
        let block = self.assemble_block()?;
        self.commit_block(block)
    }

    pub fn assemble_block(&mut self) -> Result<Block, NodeError> {
        let parent = self.head().clone();
        let take = self.mempool.len().min(self.limits.max_block_commands);
        let candidates: Vec<Command> = self.mempool.iter().take(take).cloned().collect();

        let mut trial_state = self.state.clone();
        let mut all_events = Vec::new();
        let mut commands: Vec<Command> = Vec::with_capacity(candidates.len());
        let mut rejected: Vec<Hash> = Vec::new();
        let proposer = self.proposer.clone();
        let price = Self::base_price(&trial_state, self.fee_policy);
        let mut block_gas = 0u64;
        for cmd in candidates {
            let mut probe = trial_state.clone();
            let receipt = match self.cascade.apply(cmd.clone(), &mut probe) {
                Ok(r) => r,
                Err(_) => {
                    rejected.push(cmd.id());
                    continue;
                }
            };
            if block_gas.saturating_add(receipt.total_cost) > self.limits.max_block_gas {
                break;
            }
            trial_state = probe;
            Self::charge_fee(
                self.fee_policy,
                price,
                &mut trial_state,
                &cmd.submitter,
                &proposer,
                receipt.total_cost,
            );
            block_gas = block_gas.saturating_add(receipt.total_cost);
            all_events.extend(receipt.events);
            commands.push(cmd);
        }
        Self::adjust_base_price(&mut trial_state, self.fee_policy, block_gas);

        if !rejected.is_empty() {
            let drop: std::collections::HashSet<Hash> = rejected.into_iter().collect();
            self.mempool.retain(|c| !drop.contains(&c.id()));
        }

        let mut block = Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: trial_state.root(),
            timestamp: now().max(parent.timestamp),
            proposer: self.proposer.clone(),
            events: all_events,
            commands,
        };
        block.events_root = block.compute_events_root();
        Ok(block)
    }

    pub fn commit_block(&mut self, mut block: Block) -> Result<BlockSummary, NodeError> {
        if block.parent != self.head().hash() || block.height != self.head().height + 1 {
            return Err(NodeError::BadParent);
        }

        let parent_ts = self.head().timestamp;
        let future_bound = now().saturating_add(MAX_FUTURE_DRIFT_SECS);
        if block.timestamp < parent_ts || block.timestamp > future_bound {
            return Err(NodeError::BadTimestamp {
                got: block.timestamp,
                parent: parent_ts,
            });
        }

        let mut new_state = self.state.clone();
        let mut events = Vec::new();
        let block_proposer = block.proposer.clone();
        let price = Self::base_price(&new_state, self.fee_policy);
        let mut block_gas = 0u64;
        for cmd in &block.commands {
            let receipt = self.cascade.apply(cmd.clone(), &mut new_state)?;
            Self::charge_fee(
                self.fee_policy,
                price,
                &mut new_state,
                &cmd.submitter,
                &block_proposer,
                receipt.total_cost,
            );
            block_gas = block_gas.saturating_add(receipt.total_cost);
            events.extend(receipt.events);
        }
        if block_gas > self.limits.max_block_gas {
            return Err(NodeError::BlockGasExceeded {
                used: block_gas,
                limit: self.limits.max_block_gas,
            });
        }
        Self::adjust_base_price(&mut new_state, self.fee_policy, block_gas);

        let recomputed_events_root = {
            let leaves: Vec<Hash> = events.iter().map(|e| e.commitment()).collect();
            veilux_kernel::merkle_root_of(&leaves)
        };
        if recomputed_events_root != block.events_root {
            return Err(NodeError::RootMismatch("events_root".into()));
        }
        if new_state.root() != block.state_root {
            return Err(NodeError::RootMismatch("state_root".into()));
        }

        block.events = events;
        let projection = project_block(&block, &self.keyrings)?;
        let mut delivered = 0usize;
        for keyring in &self.keyrings {
            if let Some(views) = projection.views_by_party.get(keyring.party()) {
                if let Some(ledger) = self.sub_ledgers.get_mut(keyring.party()) {
                    delivered +=
                        ledger.apply_views(block.height, projection.events_root, views, keyring)?;
                }
            }
        }

        self.state = new_state;
        let included: std::collections::HashSet<Hash> =
            block.commands.iter().map(|c| c.id()).collect();
        self.mempool.retain(|c| !included.contains(&c.id()));

        let summary = BlockSummary {
            height: block.height,
            hash: block.hash(),
            events: block.events.len(),
            events_root: block.events_root,
            state_root: block.state_root,
            total_cost: 0,
            views_delivered: delivered,
        };

        self.blocks.push(block);

        if let Some(store) = &self.store {
            let last = self.blocks.last().expect("just pushed");
            store
                .append_block(last)
                .map_err(|e| NodeError::Store(e.to_string()))?;
            store
                .save_state(&self.state)
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }

        Ok(summary)
    }

    pub fn accept_external_block(&mut self, block: Block) -> Result<bool, NodeError> {
        if block.parent != self.head().hash() || block.height != self.head().height + 1 {
            return Ok(false);
        }
        if self.blocks.iter().any(|b| b.hash() == block.hash()) {
            return Ok(false);
        }
        self.commit_block(block)?;
        Ok(true)
    }

    pub fn sub_ledger(&self, party: &PartyId) -> Option<&SubLedger> {
        self.sub_ledgers.get(party)
    }

    pub fn current_base_price(&self) -> u128 {
        Self::base_price(&self.state, self.fee_policy)
    }
}

#[derive(Debug, Clone)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: Hash,
    pub events: usize,
    pub events_root: Hash,
    pub state_root: Hash,
    pub total_cost: u64,
    pub views_delivered: usize,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::{Cascade, Visibility};
    use veilux_veil::PartyIdentity;

    fn node() -> Node {
        let mut cascade = Cascade::new();
        cascade.install(Box::new(prism_token::TokenPrism::new()));
        Node::new(PartyId::new("v0"), cascade)
    }

    #[test]
    fn system_account_cannot_submit_commands() {
        let mut n = node();
        let attacker = PartyIdentity::from_seed("attacker", &[9u8; 32]);
        for victim in ["staking/escrow", "staking/rewards", "token/meta", ""] {
            let cmd = prism_token::transfer_command(
                PartyId::new(victim),
                Visibility::Public,
                0,
                prism_token::native_token_id(),
                PartyId::new("attacker"),
                1_000,
            );
            let signed = attacker.sign(cmd);
            assert!(
                matches!(n.submit_signed(signed), Err(NodeError::ReservedAccount(_))),
                "submitter '{victim}' must be rejected"
            );
        }
    }

    #[test]
    fn normal_party_still_submits() {
        let mut n = node();
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
        let cmd = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            18,
            1_000,
            true,
        );
        assert!(n.submit_signed(alice.sign(cmd)).is_ok());
    }

    #[test]
    fn poison_command_does_not_stall_block_production() {
        let mut n = node();
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);

        let create = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            0,
            100,
            true,
        );
        n.submit_signed(alice.sign(create)).unwrap();
        let token = veilux_kernel::Hash::commit("token/id", &[b"alice", b"GLD", b"Gold"]);

        let poison = prism_token::transfer_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            token,
            PartyId::new("bob"),
            1_000_000,
        );
        n.submit_signed(alice.sign(poison)).unwrap();

        let good = prism_token::transfer_command(
            PartyId::new("alice"),
            Visibility::Public,
            2,
            token,
            PartyId::new("carol"),
            40,
        );
        n.submit_signed(alice.sign(good)).unwrap();

        let summary = n.produce_block().expect("block must be produced");
        assert!(summary.height >= 1);
        assert_eq!(
            prism_token::balance_of(&n.state, &token, &PartyId::new("carol")),
            40
        );
        assert!(n.mempool.is_empty(), "poison command must be dropped");
    }

    #[test]
    fn block_rejects_bad_timestamp() {
        let mut n = node();
        let parent = n.head().clone();
        if parent.timestamp > 0 {
            let mut block = veilux_kernel::Block {
                height: parent.height + 1,
                parent: parent.hash(),
                events_root: Hash::ZERO,
                state_root: n.state.root(),
                timestamp: parent.timestamp - 1,
                proposer: PartyId::new("v0"),
                events: vec![],
                commands: vec![],
            };
            block.events_root = block.compute_events_root();
            assert!(matches!(
                n.commit_block(block),
                Err(NodeError::BadTimestamp { .. })
            ));
        }

        let mut future = veilux_kernel::Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: n.state.root(),
            timestamp: now() + 100_000,
            proposer: PartyId::new("v0"),
            events: vec![],
            commands: vec![],
        };
        future.events_root = future.compute_events_root();
        assert!(matches!(
            n.commit_block(future),
            Err(NodeError::BadTimestamp { .. })
        ));
    }

    #[test]
    fn block_gas_limit_caps_inclusion() {
        let mut n = node();
        n.limits.max_block_gas = 6_000;
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
        for nonce in 0..5u64 {
            let cmd = prism_token::create_command(
                PartyId::new("alice"),
                Visibility::Public,
                nonce,
                &format!("T{nonce}"),
                &format!("T{nonce}"),
                0,
                1_000,
                false,
            );
            n.submit_signed(alice.sign(cmd)).unwrap();
        }
        let block = n.assemble_block().unwrap();
        assert!(
            block.commands.len() <= 1,
            "create costs 5000 gas; only one fits under a 6000 gas limit, got {}",
            block.commands.len()
        );
    }
}
