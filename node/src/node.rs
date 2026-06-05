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
    #[error("command too large: {0} bytes")]
    TooLarge(usize),
    #[error("mempool is full")]
    MempoolFull,
    #[error("store error: {0}")]
    Store(String),
    #[error("block parent/height does not extend the current head")]
    BadParent,
    #[error("{0} mismatch: block does not match re-executed result")]
    RootMismatch(String),
}

pub struct Limits {
    pub max_payload_bytes: usize,
    pub max_block_commands: usize,
    pub max_mempool: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_payload_bytes: 1024 * 1024,
            max_block_commands: 10_000,
            max_mempool: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FeePolicy {
    pub price_per_gas: u128,
    pub burn_bps: u16,
}

impl Default for FeePolicy {
    fn default() -> Self {
        FeePolicy {
            price_per_gas: 0,
            burn_bps: 5_000,
        }
    }
}

impl FeePolicy {
    pub fn enabled(&self) -> bool {
        self.price_per_gas > 0
    }
}

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

    fn charge_fee(
        policy: FeePolicy,
        state: &mut StateTree,
        payer: &PartyId,
        proposer: &PartyId,
        gas_used: u64,
    ) {
        if !policy.enabled() {
            return;
        }
        let fee = policy.price_per_gas.saturating_mul(gas_used as u128);
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
        let commands: Vec<Command> = self.mempool.iter().take(take).cloned().collect();

        let mut trial_state = self.state.clone();
        let mut all_events = Vec::new();
        let proposer = self.proposer.clone();
        for cmd in &commands {
            let receipt = self.cascade.apply(cmd.clone(), &mut trial_state)?;
            Self::charge_fee(
                self.fee_policy,
                &mut trial_state,
                &cmd.submitter,
                &proposer,
                receipt.total_cost,
            );
            all_events.extend(receipt.events);
        }

        let mut block = Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: trial_state.root(),
            timestamp: now(),
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

        let mut new_state = self.state.clone();
        let mut events = Vec::new();
        let block_proposer = block.proposer.clone();
        for cmd in &block.commands {
            let receipt = self.cascade.apply(cmd.clone(), &mut new_state)?;
            Self::charge_fee(
                self.fee_policy,
                &mut new_state,
                &cmd.submitter,
                &block_proposer,
                receipt.total_cost,
            );
            events.extend(receipt.events);
        }

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
