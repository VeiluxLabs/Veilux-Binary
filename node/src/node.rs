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
    pub store: Option<Store>,
}

impl Node {
    pub fn new(proposer: PartyId, cascade: Cascade) -> Self {
        let genesis = Block::genesis(proposer.clone(), now());
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

    pub fn head(&self) -> &Block {
        self.blocks.last().expect("chain always has genesis")
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

        let parent = self.head().clone();
        let take = self.mempool.len().min(self.limits.max_block_commands);
        let commands: Vec<Command> = self.mempool.drain(..take).collect();

        let mut all_events = Vec::new();
        let mut total_cost = 0u64;
        for cmd in commands {
            let receipt = self.cascade.apply(cmd, &mut self.state)?;
            total_cost += receipt.total_cost;
            all_events.extend(receipt.events);
        }

        let mut block = Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: self.state.root(),
            timestamp: now(),
            proposer: self.proposer.clone(),
            events: all_events,
        };
        block.events_root = block.compute_events_root();

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

        let summary = BlockSummary {
            height: block.height,
            hash: block.hash(),
            events: block.events.len(),
            events_root: block.events_root,
            state_root: block.state_root,
            total_cost,
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
