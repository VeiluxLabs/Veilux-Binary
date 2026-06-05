use serde::{Deserialize, Serialize};

use crate::crypto::Hash;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PartyId(pub String);

impl PartyId {
    pub fn new(s: impl Into<String>) -> Self {
        PartyId(s.into())
    }

    pub fn id_hash(&self) -> Hash {
        Hash::commit("party", &[self.0.as_bytes()])
    }
}

impl std::fmt::Debug for PartyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "party:{}", self.0)
    }
}

impl std::fmt::Display for PartyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Parties(Vec<PartyId>),
}

impl Visibility {
    pub fn includes(&self, party: &PartyId) -> bool {
        match self {
            Visibility::Public => true,
            Visibility::Parties(set) => set.contains(party),
        }
    }

    pub fn stakeholders(&self) -> &[PartyId] {
        match self {
            Visibility::Public => &[],
            Visibility::Parties(set) => set,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub prism: String,
    pub submitter: PartyId,
    pub visibility: Visibility,
    pub payload: Vec<u8>,
    pub nonce: u64,
}

impl Command {
    pub fn id(&self) -> Hash {
        Hash::commit(
            "command",
            &[
                self.prism.as_bytes(),
                self.submitter.0.as_bytes(),
                &self.nonce.to_le_bytes(),
                &self.payload,
            ],
        )
    }

    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(64 + self.payload.len());
        v.extend_from_slice(b"veilux/command/v1");
        v.push(0xff);
        v.extend_from_slice(self.prism.as_bytes());
        v.push(0xff);
        v.extend_from_slice(self.submitter.0.as_bytes());
        v.push(0xff);
        v.extend_from_slice(&self.nonce.to_le_bytes());
        let vis = serde_json::to_vec(&self.visibility).unwrap_or_default();
        v.extend_from_slice(&vis);
        v.push(0xff);
        v.extend_from_slice(&self.payload);
        v
    }

    pub fn signing_bytes_for_chain(&self, chain_id: u64) -> Vec<u8> {
        let mut v = self.signing_bytes();
        if chain_id != 0 {
            v.push(0xff);
            v.extend_from_slice(b"chain");
            v.extend_from_slice(&chain_id.to_le_bytes());
        }
        v
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedCommand {
    pub command: Command,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
    #[serde(default)]
    pub chain_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub source_command: Hash,
    pub prism: String,
    pub visibility: Visibility,
    pub payload: Vec<u8>,
}

impl Event {
    pub fn id(&self) -> Hash {
        let vis = serde_json::to_vec(&self.visibility).unwrap_or_default();
        Hash::commit(
            "event",
            &[
                self.source_command.as_bytes(),
                self.prism.as_bytes(),
                &vis,
                &self.payload,
            ],
        )
    }

    pub fn commitment(&self) -> Hash {
        self.id()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub height: u64,
    pub parent: Hash,
    pub events_root: Hash,
    pub state_root: Hash,
    pub timestamp: u64,
    pub proposer: PartyId,
    pub events: Vec<Event>,
    #[serde(default)]
    pub commands: Vec<Command>,
}

impl Block {
    pub fn hash(&self) -> Hash {
        Hash::commit(
            "block",
            &[
                &self.height.to_le_bytes(),
                self.parent.as_bytes(),
                self.events_root.as_bytes(),
                self.state_root.as_bytes(),
                self.commands_root().as_bytes(),
                &self.timestamp.to_le_bytes(),
                self.proposer.0.as_bytes(),
            ],
        )
    }

    pub fn compute_events_root(&self) -> Hash {
        let leaves: Vec<Hash> = self.events.iter().map(|e| e.commitment()).collect();
        crate::crypto::merkle_root(&leaves)
    }

    pub fn commands_root(&self) -> Hash {
        let leaves: Vec<Hash> = self.commands.iter().map(|c| c.id()).collect();
        crate::crypto::merkle_root(&leaves)
    }

    pub fn genesis(proposer: PartyId, timestamp: u64) -> Self {
        Block {
            height: 0,
            parent: Hash::ZERO,
            events_root: Hash::ZERO,
            state_root: Hash::ZERO,
            timestamp,
            proposer,
            events: vec![],
            commands: vec![],
        }
    }

    pub fn deterministic_genesis() -> Self {
        Block {
            height: 0,
            parent: Hash::ZERO,
            events_root: Hash::ZERO,
            state_root: Hash::ZERO,
            timestamp: 0,
            proposer: PartyId::new("genesis"),
            events: vec![],
            commands: vec![],
        }
    }
}
