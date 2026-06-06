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
    #[error("wrong chain id: command signed for {got}, this chain is {expected}")]
    WrongChainId { got: u64, expected: u64 },
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
    #[error("private envelope commitment does not match its sealed shares")]
    BadPrivateCommitment,
    #[error("private envelope already applied")]
    DuplicatePrivateCommitment,
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
    pub private_state: StateTree,
    pub private_commitments: Vec<Hash>,
    pub private_envelopes: std::collections::HashMap<Hash, veilux_veil::PrivateEnvelope>,
    pub attestations: veilux_veil::AttestationBook,
    pub blocks: Vec<Block>,
    pub mempool: Vec<Command>,
    pub keyrings: Vec<ViewKeyring>,
    pub sub_ledgers: HashMap<PartyId, SubLedger>,
    pub proposer: PartyId,
    pub accounts: HashMap<PartyId, Vec<u8>>,
    pub nonces: HashMap<PartyId, u64>,
    pub limits: Limits,
    pub fee_policy: FeePolicy,
    pub chain_id: u64,
    pub store: Option<Store>,
}

impl Node {
    pub fn new(proposer: PartyId, cascade: Cascade) -> Self {
        let genesis = Block::deterministic_genesis();
        Node {
            cascade,
            state: StateTree::new(),
            private_state: StateTree::new(),
            private_commitments: Vec::new(),
            private_envelopes: std::collections::HashMap::new(),
            attestations: veilux_veil::AttestationBook::default(),
            blocks: vec![genesis],
            mempool: Vec::new(),
            keyrings: Vec::new(),
            sub_ledgers: HashMap::new(),
            proposer,
            accounts: HashMap::new(),
            nonces: HashMap::new(),
            limits: Limits::default(),
            fee_policy: FeePolicy::default(),
            chain_id: 0,
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
        if let Some(pstate) = store
            .load_private_state()
            .map_err(|e| NodeError::Store(e.to_string()))?
        {
            node.private_state = pstate;
        }
        node.private_commitments = store
            .load_private_commitments()
            .map_err(|e| NodeError::Store(e.to_string()))?;
        node.store = Some(store);
        node.restore_mempool()?;
        Ok(node)
    }

    fn restore_mempool(&mut self) -> Result<(), NodeError> {
        let pending = match &self.store {
            Some(store) => store
                .load_pending()
                .map_err(|e| NodeError::Store(e.to_string()))?,
            None => return Ok(()),
        };
        if pending.is_empty() {
            return Ok(());
        }
        let mut restored = 0usize;
        for signed in pending {
            if self.ingest_signed(signed, false).is_ok() {
                restored += 1;
            }
        }
        if let Some(store) = &self.store {
            let alive: std::collections::HashSet<Hash> =
                self.mempool.iter().map(|c| c.id()).collect();
            let kept: Vec<SignedCommand> = store
                .load_pending()
                .map_err(|e| NodeError::Store(e.to_string()))?
                .into_iter()
                .filter(|s| alive.contains(&s.command.id()))
                .collect();
            store
                .rewrite_pending(&kept)
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }
        tracing::info!(restored, "pending transactions restored from disk");
        Ok(())
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
        self.ingest_signed(signed, true)
    }

    fn ingest_signed(&mut self, signed: SignedCommand, persist: bool) -> Result<(), NodeError> {
        let cmd = &signed.command;

        if cmd.payload.len() > self.limits.max_payload_bytes {
            return Err(NodeError::TooLarge(cmd.payload.len()));
        }

        if cmd.submitter.0.contains('/') || cmd.submitter.0.is_empty() {
            return Err(NodeError::ReservedAccount(cmd.submitter.0.clone()));
        }

        if signed.chain_id != self.chain_id {
            return Err(NodeError::WrongChainId {
                got: signed.chain_id,
                expected: self.chain_id,
            });
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
        if persist {
            if let Some(store) = &self.store {
                store
                    .append_pending(&signed)
                    .map_err(|e| NodeError::Store(e.to_string()))?;
            }
        }
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
        let block_ts = now().max(parent.timestamp);
        let _ = trial_state.put_json("chain/now", &block_ts);
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
            self.sync_persisted_mempool()?;
        }

        let mut block = Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: trial_state.root(),
            timestamp: block_ts,
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
        let _ = new_state.put_json("chain/now", &block.timestamp);
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
        self.sync_persisted_mempool()?;

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

    pub fn private_root(&self) -> Hash {
        self.private_state.root()
    }

    pub fn apply_private_envelope(
        &mut self,
        envelope: &veilux_veil::PrivateEnvelope,
    ) -> Result<PrivateOutcome, NodeError> {
        if !envelope.verify_commitment() {
            return Err(NodeError::BadPrivateCommitment);
        }
        if self.private_commitments.contains(&envelope.commitment) {
            return Err(NodeError::DuplicatePrivateCommitment);
        }

        self.private_commitments.push(envelope.commitment);
        self.private_envelopes
            .insert(envelope.commitment, envelope.clone());
        if let Some(store) = &self.store {
            store
                .save_private_commitments(&self.private_commitments)
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }

        let keyring = self
            .keyrings
            .iter()
            .find(|k| envelope.is_stakeholder(k.party()))
            .cloned();

        let mut executed = false;
        let mut attestation = None;
        if let Some(kr) = keyring {
            let inner = veilux_veil::open_private(envelope, &kr)?;
            if !self.cascade.has(&inner.prism) {
                return Err(NodeError::UnknownPrism(inner.prism));
            }
            let receipt = self.cascade.apply(inner, &mut self.private_state)?;
            let _ = receipt;
            executed = true;
            let private_root = self.private_state.root();
            let mut seed = [0u8; 32];
            let s = kr.private_seed();
            seed[..s.len().min(32)].copy_from_slice(&s[..s.len().min(32)]);
            let identity = veilux_veil::PartyIdentity::from_seed(&kr.party().0, &seed);
            let att =
                veilux_veil::RootAttestation::create(&identity, envelope.commitment, private_root);
            let _ = self.attestations.record(att.clone());
            attestation = Some(att);
            if let Some(store) = &self.store {
                store
                    .save_private_state(&self.private_state)
                    .map_err(|e| NodeError::Store(e.to_string()))?;
            }
        }

        Ok(PrivateOutcome {
            commitment: envelope.commitment,
            executed,
            private_root: self.private_state.root(),
            attestation,
        })
    }

    pub fn record_attestation(
        &mut self,
        attestation: veilux_veil::RootAttestation,
    ) -> veilux_veil::AttestationOutcome {
        self.attestations.record(attestation)
    }

    pub fn private_divergence_proof(
        &self,
        attestation: &veilux_veil::RootAttestation,
    ) -> Option<prism_staking::EquivocationProof> {
        let pair = self.attestations.self_equivocation(attestation)?;
        Some(prism_staking::EquivocationProof {
            offender: pair.party,
            public_key: hex::encode(&pair.public_key),
            message_a: hex::encode(pair.a.signed_message()),
            signature_a: hex::encode(&pair.a.signature),
            message_b: hex::encode(pair.b.signed_message()),
            signature_b: hex::encode(&pair.b.signature),
        })
    }

    pub fn private_quorum_fraud_proofs(
        &self,
        commitment: &Hash,
    ) -> Vec<prism_staking::QuorumFraudProof> {
        let envelope = match self.private_envelopes.get(commitment) {
            Some(e) => e.clone(),
            None => return Vec::new(),
        };
        let count = envelope.stakeholders.len();
        let canonical = match self.attestations.canonical_root(commitment, count) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let all = self.attestations.agreement(commitment);
        let majority: Vec<prism_staking::AttestationStatement> = all
            .iter()
            .filter(|e| e.private_root == canonical)
            .map(|e| prism_staking::AttestationStatement {
                party: e.party.clone(),
                public_key: hex::encode(&e.public_key),
                signature: hex::encode(&e.signature),
                root: e.private_root.to_hex(),
            })
            .collect();

        self.attestations
            .minority_offenders(commitment, count)
            .into_iter()
            .map(|o| prism_staking::QuorumFraudProof {
                offender: o.party.clone(),
                envelope: envelope.clone(),
                offender_stmt: prism_staking::AttestationStatement {
                    party: o.party.clone(),
                    public_key: hex::encode(&o.public_key),
                    signature: hex::encode(&o.signature),
                    root: o.private_root.to_hex(),
                },
                majority: majority.clone(),
            })
            .collect()
    }

    pub fn private_consistent(&self, commitment: &Hash) -> bool {
        self.attestations.is_consistent(commitment)
    }

    fn sync_persisted_mempool(&self) -> Result<(), NodeError> {
        if let Some(store) = &self.store {
            let alive: std::collections::HashSet<Hash> =
                self.mempool.iter().map(|c| c.id()).collect();
            let kept: Vec<SignedCommand> = store
                .load_pending()
                .map_err(|e| NodeError::Store(e.to_string()))?
                .into_iter()
                .filter(|s| alive.contains(&s.command.id()))
                .collect();
            store
                .rewrite_pending(&kept)
                .map_err(|e| NodeError::Store(e.to_string()))?;
        }
        Ok(())
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

#[derive(Debug, Clone)]
pub struct PrivateOutcome {
    pub commitment: Hash,
    pub executed: bool,
    pub private_root: Hash,
    pub attestation: Option<veilux_veil::RootAttestation>,
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

    fn full_node() -> Node {
        let mut cascade = Cascade::new();
        cascade.install(Box::new(prism_token::TokenPrism::new()));
        cascade.install(Box::new(prism_multisig::MultisigPrism::new()));
        cascade.install(Box::new(prism_vesting::VestingPrism::new()));
        cascade.install(Box::new(prism_dex::DexPrism::new()));
        Node::new(PartyId::new("v0"), cascade)
    }

    fn derive_token_id(party: &str, symbol: &str, name: &str) -> Hash {
        Hash::commit(
            "token/id",
            &[party.as_bytes(), symbol.as_bytes(), name.as_bytes()],
        )
    }

    #[test]
    fn multisig_dispatches_inner_transfer_through_cascade() {
        let mut n = full_node();
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
        let bob = PartyIdentity::from_seed("bob", &[2u8; 32]);

        let create = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            0,
            1_000,
            false,
        );
        n.submit_signed(alice.sign(create)).unwrap();
        n.produce_block().unwrap();
        let token = derive_token_id("alice", "GLD", "Gold");

        let acc_cmd = prism_multisig::create_account_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            vec![PartyId::new("alice"), PartyId::new("bob")],
            2,
        );
        n.submit_signed(alice.sign(acc_cmd)).unwrap();
        n.produce_block().unwrap();
        let account = n
            .head()
            .events
            .iter()
            .find_map(|e| {
                match serde_json::from_slice::<prism_multisig::MultisigEvent>(&e.payload).ok()? {
                    prism_multisig::MultisigEvent::AccountCreated { account, .. } => Some(account),
                    _ => None,
                }
            })
            .expect("account created");

        prism_token::credit(&mut n.state, &token, &PartyId::new("multisig"), 500).unwrap();

        let inner = prism_token::transfer_command(
            PartyId::new("multisig"),
            Visibility::Public,
            0,
            token,
            PartyId::new("carol"),
            500,
        );
        let propose = prism_multisig::propose_command(
            PartyId::new("alice"),
            Visibility::Public,
            2,
            account,
            inner,
        );
        n.submit_signed(alice.sign(propose)).unwrap();
        n.produce_block().unwrap();
        assert_eq!(
            prism_token::balance_of(&n.state, &token, &PartyId::new("carol")),
            0,
            "one approval must not move funds in a 2-of-2"
        );

        let confirm =
            prism_multisig::confirm_command(PartyId::new("bob"), Visibility::Public, 0, account, 0);
        n.submit_signed(bob.sign(confirm)).unwrap();
        n.produce_block().unwrap();
        assert_eq!(
            prism_token::balance_of(&n.state, &token, &PartyId::new("carol")),
            500,
            "second approval must cascade the inner transfer"
        );
    }

    #[test]
    fn dex_swap_through_block_production() {
        let mut n = full_node();
        let lp = PartyIdentity::from_seed("lp", &[3u8; 32]);

        let mk = |sym: &str, nonce: u64| {
            prism_token::create_command(
                PartyId::new("lp"),
                Visibility::Public,
                nonce,
                sym,
                sym,
                0,
                1_000_000,
                false,
            )
        };
        n.submit_signed(lp.sign(mk("AAA", 0))).unwrap();
        n.produce_block().unwrap();
        let a = derive_token_id("lp", "AAA", "AAA");
        n.submit_signed(lp.sign(mk("BBB", 1))).unwrap();
        n.produce_block().unwrap();
        let b = derive_token_id("lp", "BBB", "BBB");

        let create_pool =
            prism_dex::create_pool_command(PartyId::new("lp"), Visibility::Public, 2, a, b);
        n.submit_signed(lp.sign(create_pool)).unwrap();
        n.produce_block().unwrap();
        let pool = n
            .head()
            .events
            .iter()
            .find_map(|e| {
                match serde_json::from_slice::<prism_dex::DexEvent>(&e.payload).ok()? {
                    prism_dex::DexEvent::PoolCreated { pool, .. } => Some(pool),
                    _ => None,
                }
            })
            .expect("pool created");

        let add = prism_dex::add_liquidity_command(
            PartyId::new("lp"),
            Visibility::Public,
            3,
            pool,
            100_000,
            100_000,
        );
        n.submit_signed(lp.sign(add)).unwrap();
        n.produce_block().unwrap();

        let before = prism_token::balance_of(&n.state, &b, &PartyId::new("lp"));
        let swap = prism_dex::swap_command(
            PartyId::new("lp"),
            Visibility::Public,
            4,
            pool,
            a,
            10_000,
            1,
        );
        n.submit_signed(lp.sign(swap)).unwrap();
        n.produce_block().unwrap();
        let after = prism_token::balance_of(&n.state, &b, &PartyId::new("lp"));
        assert!(after > before, "swap must credit token B to the trader");
        let pool_state = prism_dex::pool_of(&n.state, &pool).unwrap();
        assert_eq!(pool_state.reserve_a, 110_000, "pool reserve A grows by amount_in");
    }

    #[test]
    fn vesting_releases_with_chain_time() {
        let mut n = full_node();
        let funder = PartyIdentity::from_seed("funder", &[4u8; 32]);
        let create = prism_token::create_command(
            PartyId::new("funder"),
            Visibility::Public,
            0,
            "Vest",
            "VST",
            0,
            1_000_000,
            false,
        );
        n.submit_signed(funder.sign(create)).unwrap();
        n.produce_block().unwrap();
        let token = derive_token_id("funder", "VST", "Vest");

        let create_sched = prism_vesting::create_command(
            PartyId::new("funder"),
            Visibility::Public,
            1,
            token,
            PartyId::new("team"),
            100_000,
            0,
            0,
            1,
            false,
        );
        n.submit_signed(funder.sign(create_sched)).unwrap();
        n.produce_block().unwrap();
        let schedule = n
            .head()
            .events
            .iter()
            .find_map(|e| {
                match serde_json::from_slice::<prism_vesting::VestingEvent>(&e.payload).ok()? {
                    prism_vesting::VestingEvent::Created { schedule, .. } => Some(schedule),
                    _ => None,
                }
            })
            .expect("schedule created");

        let release = prism_vesting::release_command(
            PartyId::new("team"),
            Visibility::Public,
            0,
            schedule,
        );
        let team = PartyIdentity::from_seed("team", &[5u8; 32]);
        n.submit_signed(team.sign(release)).unwrap();
        n.produce_block().unwrap();
        assert!(
            prism_token::balance_of(&n.state, &token, &PartyId::new("team")) > 0,
            "fully-vested (duration=1, start=0) schedule must release to the beneficiary"
        );
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
    fn pending_transactions_survive_restart() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("veilux-mempool-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
        let make = |nonce: u64| {
            prism_token::create_command(
                PartyId::new("alice"),
                Visibility::Public,
                nonce,
                &format!("T{nonce}"),
                &format!("T{nonce}"),
                0,
                1_000,
                true,
            )
        };

        {
            let mut cascade = Cascade::new();
            cascade.install(Box::new(prism_token::TokenPrism::new()));
            let store = Store::open(&dir).unwrap();
            let mut n = Node::with_store(PartyId::new("v0"), cascade, store).unwrap();
            n.submit_signed(alice.sign(make(0))).unwrap();
            n.submit_signed(alice.sign(make(1))).unwrap();
            assert_eq!(n.mempool.len(), 2);
        }

        let mut cascade = Cascade::new();
        cascade.install(Box::new(prism_token::TokenPrism::new()));
        let store = Store::open(&dir).unwrap();
        let mut n = Node::with_store(PartyId::new("v0"), cascade, store).unwrap();
        assert_eq!(
            n.mempool.len(),
            2,
            "both pending transactions must be restored from disk"
        );

        let summary = n.produce_block().unwrap();
        assert_eq!(summary.height, 1);
        assert!(n.mempool.is_empty());

        let store2 = Store::open(&dir).unwrap();
        assert!(
            store2.load_pending().unwrap().is_empty(),
            "persisted mempool log must be pruned after the block is committed"
        );

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn wrong_chain_id_is_rejected() {
        let mut n = node();
        n.chain_id = 42;
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
        let cmd = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            0,
            1_000,
            true,
        );
        let wrong = alice.sign_for_chain(cmd.clone(), 7);
        assert!(matches!(
            n.submit_signed(wrong),
            Err(NodeError::WrongChainId { .. })
        ));
        let right = alice.sign_for_chain(cmd, 42);
        assert!(n.submit_signed(right).is_ok());
    }

    #[test]
    #[ignore = "perf benchmark; run with --ignored --nocapture"]
    fn tps_benchmark() {
        use std::time::Instant;

        let mut cascade = Cascade::new();
        cascade.install(Box::new(prism_token::TokenPrism::new()));
        let mut n = Node::new(PartyId::new("v0"), cascade);
        n.limits.max_block_commands = 200_000;
        n.limits.max_block_gas = u64::MAX;
        n.limits.max_mempool = 500_000;

        let n_tx: usize = 20_000;
        let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);

        let token = prism_token::seed_native_token(
            &mut n.state,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("alice"),
            &[(PartyId::new("alice"), (n_tx as u128) * 10)],
        )
        .unwrap();

        let signed: Vec<_> = (0..n_tx)
            .map(|i| {
                let cmd = prism_token::transfer_command(
                    PartyId::new("alice"),
                    Visibility::Public,
                    i as u64,
                    token,
                    PartyId::new("bob"),
                    1,
                );
                alice.sign(cmd)
            })
            .collect();

        let t_submit = Instant::now();
        for s in signed.clone() {
            n.submit_signed(s).unwrap();
        }
        let submit_secs = t_submit.elapsed().as_secs_f64();

        let t_batch = Instant::now();
        let results = veilux_veil::verify_signed_batch(&signed);
        let batch_secs = t_batch.elapsed().as_secs_f64();
        assert!(results.iter().all(|r| r.is_ok()));
        let batch_tps = n_tx as f64 / batch_secs;

        let t_block = Instant::now();
        let summary = n.produce_block().unwrap();
        let block_secs = t_block.elapsed().as_secs_f64();

        let included = summary.events;
        let exec_tps = included as f64 / block_secs;
        let ingest_tps = n_tx as f64 / submit_secs;
        let e2e_tps = n_tx as f64 / (submit_secs + block_secs);

        println!(
            "\n=== VEILUX TPS (single-node, in-memory, token transfer) ===\n\
             tx                : {n_tx}\n\
             included in block : {included}\n\
             ingest serial (verify+mempool) : {ingest_tps:.0} tx/s ({submit_secs:.3}s)\n\
             verify batch (parallel)        : {batch_tps:.0} tx/s ({batch_secs:.3}s)\n\
             execute+state_root             : {exec_tps:.0} tx/s ({block_secs:.3}s)\n\
             end-to-end                     : {e2e_tps:.0} tx/s\n"
        );

        assert_eq!(included, n_tx);
        assert_eq!(
            prism_token::balance_of(&n.state, &token, &PartyId::new("bob")),
            n_tx as u128
        );
    }

    #[test]
    fn confidential_tx_executes_only_on_stakeholder_nodes() {
        use veilux_veil::{seal_private, ViewKeyring};

        let alice_ring = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-private");
        let bob_ring = ViewKeyring::from_passphrase(PartyId::new("bob"), "bob-private");

        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(vec![PartyId::new("alice"), PartyId::new("bob")]),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let stakeholders = vec![PartyId::new("alice"), PartyId::new("bob")];
        let envelope = seal_private(
            &inner,
            &stakeholders,
            &[alice_ring.clone(), bob_ring.clone()],
            Hash::digest(b"confidential-round-1"),
        )
        .unwrap();

        let mut stakeholder_node = node();
        stakeholder_node.host_party(alice_ring);
        let global_root_before = stakeholder_node.state.root();
        let out = stakeholder_node
            .apply_private_envelope(&envelope)
            .expect("stakeholder applies envelope");
        assert!(
            out.executed,
            "a stakeholder node must execute the inner command"
        );
        assert_ne!(
            stakeholder_node.private_root(),
            Hash::ZERO,
            "the stakeholder's private state must change"
        );
        assert_eq!(
            stakeholder_node.state.root(),
            global_root_before,
            "a confidential tx must NOT touch the global public state root"
        );

        let mut outsider_node = node();
        let outsider_out = outsider_node
            .apply_private_envelope(&envelope)
            .expect("outsider records commitment");
        assert!(
            !outsider_out.executed,
            "a non-stakeholder node must NOT execute the inner command"
        );
        assert_eq!(
            outsider_node.private_root(),
            Hash::ZERO,
            "a non-stakeholder must learn nothing: its private state stays empty"
        );
        assert!(
            outsider_node
                .private_commitments
                .contains(&envelope.commitment),
            "the outsider still witnesses that the confidential tx happened (commitment ordered)"
        );

        assert_eq!(
            out.commitment, outsider_out.commitment,
            "both nodes agree on the public commitment"
        );
    }

    #[test]
    fn confidential_tx_rejects_tampered_commitment() {
        let alice_ring =
            veilux_veil::ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-private");
        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(vec![PartyId::new("alice")]),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let mut envelope = veilux_veil::seal_private(
            &inner,
            &[PartyId::new("alice")],
            &[alice_ring.clone()],
            Hash::digest(b"r"),
        )
        .unwrap();
        envelope.shares[0].ciphertext.push(0x00);

        let mut n = node();
        n.host_party(alice_ring);
        assert!(matches!(
            n.apply_private_envelope(&envelope),
            Err(NodeError::BadPrivateCommitment)
        ));
    }

    #[test]
    fn private_state_and_replay_guard_survive_restart() {
        use veilux_veil::{seal_private, ViewKeyring};

        let mut dir = std::env::temp_dir();
        dir.push(format!("veilux-priv-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-private");
        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(vec![PartyId::new("alice")]),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let envelope = seal_private(
            &inner,
            &[PartyId::new("alice")],
            &[alice.clone()],
            Hash::digest(b"restart-round"),
        )
        .unwrap();

        let root_after_first;
        {
            let mut cascade = Cascade::new();
            cascade.install(Box::new(prism_token::TokenPrism::new()));
            let store = Store::open(&dir).unwrap();
            let mut n = Node::with_store(PartyId::new("v0"), cascade, store).unwrap();
            n.host_party(alice.clone());
            let out = n.apply_private_envelope(&envelope).unwrap();
            assert!(out.executed);
            root_after_first = out.private_root;
            assert_ne!(root_after_first, Hash::ZERO);
        }

        let mut cascade = Cascade::new();
        cascade.install(Box::new(prism_token::TokenPrism::new()));
        let store = Store::open(&dir).unwrap();
        let mut n = Node::with_store(PartyId::new("v0"), cascade, store).unwrap();
        n.host_party(alice);

        assert_eq!(
            n.private_root(),
            root_after_first,
            "the private state root must be reloaded from disk after restart"
        );
        assert!(
            matches!(
                n.apply_private_envelope(&envelope),
                Err(NodeError::DuplicatePrivateCommitment)
            ),
            "the replay guard must reject an already-applied envelope using persisted commitments"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stakeholders_attest_matching_private_roots() {
        use veilux_veil::{seal_private, ViewKeyring};

        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-pk");
        let bob = ViewKeyring::from_passphrase(PartyId::new("bob"), "bob-pk");
        let parties = vec![PartyId::new("alice"), PartyId::new("bob")];

        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(parties.clone()),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let envelope = seal_private(
            &inner,
            &parties,
            &[alice.clone(), bob.clone()],
            Hash::digest(b"attest-round"),
        )
        .unwrap();

        let mut alice_node = node();
        alice_node.host_party(alice);
        let a_out = alice_node.apply_private_envelope(&envelope).unwrap();
        let a_att = a_out.attestation.expect("alice attests");

        let mut bob_node = node();
        bob_node.host_party(bob);
        let b_out = bob_node.apply_private_envelope(&envelope).unwrap();
        let b_att = b_out.attestation.expect("bob attests");

        assert_eq!(
            a_att.private_root, b_att.private_root,
            "two honest stakeholders executing the same confidential tx must reach the same private root"
        );

        assert_eq!(
            alice_node.record_attestation(b_att),
            veilux_veil::AttestationOutcome::Recorded,
            "alice records bob's matching attestation without divergence"
        );
        assert!(alice_node.private_consistent(&envelope.commitment));

        let evil = veilux_veil::RootAttestation::create(
            &veilux_veil::PartyIdentity::from_seed("bob", &[9u8; 32]),
            envelope.commitment,
            Hash::digest(b"forged-divergent-root"),
        );
        let outcome = alice_node.record_attestation(evil);
        assert!(
            matches!(outcome, veilux_veil::AttestationOutcome::Divergence { .. }),
            "a conflicting private root for an already-attested stakeholder must be flagged as divergence, got {outcome:?}"
        );
    }

    #[test]
    fn private_divergence_yields_a_valid_slash_proof() {
        use veilux_veil::{PartyIdentity, RootAttestation};

        let commitment = Hash::digest(b"divergent-confidential-tx");
        let bob_seed = [0x42u8; 32];
        let bob = PartyIdentity::from_seed("bob", &bob_seed);

        let honest = RootAttestation::create(&bob, commitment, Hash::digest(b"root-honest"));
        let conflicting = RootAttestation::create(&bob, commitment, Hash::digest(b"root-LIED"));

        let mut n = node();
        assert_eq!(
            n.record_attestation(honest),
            veilux_veil::AttestationOutcome::Recorded
        );

        let proof = n
            .private_divergence_proof(&conflicting)
            .expect("a same-party conflicting attestation must yield a slash proof");
        assert_eq!(proof.offender, PartyId::new("bob"));
        assert_ne!(
            proof.message_a, proof.message_b,
            "the two signed messages must differ"
        );

        let mut staking_node = {
            let mut cascade = Cascade::new();
            cascade.install(Box::new(prism_token::TokenPrism::new()));
            cascade.install(Box::new(prism_staking::StakingPrism::new()));
            Node::new(PartyId::new("v0"), cascade)
        };
        let _ = prism_token::seed_native_token(
            &mut staking_node.state,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("bob"), 10_000)],
        );
        let bob_stake = prism_staking::staking_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            1,
            &prism_staking::StakingCommand::Stake { amount: 1_000 },
        );
        let bob_id = PartyIdentity::from_seed("bob", &[1u8; 32]);
        staking_node.submit_signed(bob_id.sign(bob_stake)).unwrap();
        staking_node.produce_block().unwrap();
        let before = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("bob"));

        let slash = prism_staking::staking_command(
            PartyId::new("watchdog"),
            Visibility::Public,
            0,
            2,
            &prism_staking::StakingCommand::Slash { proof },
        );
        let watchdog = PartyIdentity::from_seed("watchdog", &[7u8; 32]);
        staking_node.submit_signed(watchdog.sign(slash)).unwrap();
        staking_node.produce_block().unwrap();
        let after = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("bob"));

        assert!(
            after < before,
            "a private-root divergence proof must actually slash the offender's stake: {before} -> {after}"
        );
    }

    #[test]
    fn quorum_minority_liar_is_slashed_end_to_end() {
        use veilux_veil::{seal_private, PartyIdentity, RootAttestation, ViewKeyring};

        let parties = vec![
            PartyId::new("alice"),
            PartyId::new("bob"),
            PartyId::new("carol"),
            PartyId::new("mallory"),
        ];
        let rings: Vec<ViewKeyring> = parties
            .iter()
            .map(|p| ViewKeyring::from_passphrase(p.clone(), &format!("{}-seed", p.0)))
            .collect();

        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(parties.clone()),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let envelope = seal_private(&inner, &parties, &rings, Hash::digest(b"q-round")).unwrap();
        let commitment = envelope.commitment;
        let honest = Hash::digest(b"true-root");
        let lie = Hash::digest(b"mallory-fabricated-root");

        let mk = |name: &str| {
            PartyIdentity::from_seed(name, &{
                let mut s = [0u8; 32];
                let b = name.as_bytes();
                s[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
                s
            })
        };

        let mut n = node();
        n.private_envelopes.insert(commitment, envelope.clone());
        for who in ["alice", "bob", "carol"] {
            assert_eq!(
                n.record_attestation(RootAttestation::create(&mk(who), commitment, honest)),
                veilux_veil::AttestationOutcome::Recorded
            );
        }
        let _ = n.record_attestation(RootAttestation::create(&mk("mallory"), commitment, lie));

        let proofs = n.private_quorum_fraud_proofs(&commitment);
        assert_eq!(proofs.len(), 1, "exactly one minority liar (mallory)");
        let proof = proofs.into_iter().next().unwrap();
        assert_eq!(proof.offender, PartyId::new("mallory"));
        assert_eq!(proof.majority.len(), 3);

        let mut staking_node = {
            let mut cascade = Cascade::new();
            cascade.install(Box::new(prism_token::TokenPrism::new()));
            cascade.install(Box::new(prism_staking::StakingPrism::new()));
            Node::new(PartyId::new("v0"), cascade)
        };
        let _ = prism_token::seed_native_token(
            &mut staking_node.state,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("mallory"), 10_000)],
        );
        let stake = prism_staking::staking_command(
            PartyId::new("mallory"),
            Visibility::Public,
            0,
            1,
            &prism_staking::StakingCommand::Stake { amount: 2_000 },
        );
        let mallory_id = mk("mallory");
        staking_node.submit_signed(mallory_id.sign(stake)).unwrap();
        staking_node.produce_block().unwrap();
        let before = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("mallory"));

        let slash = prism_staking::staking_command(
            PartyId::new("watchdog"),
            Visibility::Public,
            0,
            2,
            &prism_staking::StakingCommand::SlashQuorum { proof },
        );
        let watchdog = PartyIdentity::from_seed("watchdog", &[7u8; 32]);
        staking_node.submit_signed(watchdog.sign(slash)).unwrap();
        staking_node.produce_block().unwrap();
        let after = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("mallory"));

        assert!(
            after < before,
            "a quorum-fraud proof must slash the minority liar's stake: {before} -> {after}"
        );
    }

    #[test]
    fn forged_majority_cannot_slash_an_honest_stakeholder() {
        use veilux_veil::{seal_private, PartyIdentity, RootAttestation, ViewKeyring};

        let real_parties = vec![PartyId::new("alice"), PartyId::new("bob")];
        let rings: Vec<ViewKeyring> = real_parties
            .iter()
            .map(|p| ViewKeyring::from_passphrase(p.clone(), &format!("{}-seed", p.0)))
            .collect();
        let inner = prism_token::create_command(
            PartyId::new("alice"),
            Visibility::Parties(real_parties.clone()),
            0,
            "Secret",
            "SEC",
            0,
            1_000,
            true,
        );
        let envelope = seal_private(&inner, &real_parties, &rings, Hash::digest(b"x")).unwrap();
        let commitment = envelope.commitment;
        let true_root = Hash::digest(b"the-true-root");

        let mk = |name: &str, seed: u8| PartyIdentity::from_seed(name, &[seed; 32]);

        let alice = mk("alice", 1);
        let alice_att = RootAttestation::create(&alice, commitment, true_root);

        let fabricated_root = Hash::digest(b"attacker-fabricated");
        let sybils: Vec<prism_staking::AttestationStatement> = (0..5)
            .map(|i| {
                let s = mk(&format!("sybil{i}"), 100 + i as u8);
                let att = RootAttestation::create(&s, commitment, fabricated_root);
                prism_staking::AttestationStatement {
                    party: att.party.clone(),
                    public_key: hex::encode(&att.public_key),
                    signature: hex::encode(&att.signature),
                    root: att.private_root.to_hex(),
                }
            })
            .collect();

        let forged = prism_staking::QuorumFraudProof {
            offender: PartyId::new("alice"),
            envelope: envelope.clone(),
            offender_stmt: prism_staking::AttestationStatement {
                party: PartyId::new("alice"),
                public_key: hex::encode(&alice_att.public_key),
                signature: hex::encode(&alice_att.signature),
                root: alice_att.private_root.to_hex(),
            },
            majority: sybils,
        };

        let mut staking_node = {
            let mut cascade = Cascade::new();
            cascade.install(Box::new(prism_token::TokenPrism::new()));
            cascade.install(Box::new(prism_staking::StakingPrism::new()));
            Node::new(PartyId::new("v0"), cascade)
        };
        let _ = prism_token::seed_native_token(
            &mut staking_node.state,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("alice"), 10_000)],
        );
        let stake = prism_staking::staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            1,
            &prism_staking::StakingCommand::Stake { amount: 2_000 },
        );
        staking_node.submit_signed(alice.sign(stake)).unwrap();
        staking_node.produce_block().unwrap();
        let before = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("alice"));

        let slash = prism_staking::staking_command(
            PartyId::new("attacker"),
            Visibility::Public,
            0,
            2,
            &prism_staking::StakingCommand::SlashQuorum { proof: forged },
        );
        let attacker = PartyIdentity::from_seed("attacker", &[9u8; 32]);
        let result = staking_node.submit_signed(attacker.sign(slash));
        let _ = result;
        let _ = staking_node.produce_block();
        let after = prism_staking::voting_power_of(&staking_node.state, &PartyId::new("alice"));

        assert_eq!(
            before, after,
            "a forged majority of sybils (not real stakeholders) must NOT be able to slash an honest stakeholder"
        );
    }
}
