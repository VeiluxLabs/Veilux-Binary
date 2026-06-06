pub mod validator;
pub mod vote;

pub use validator::{Validator, ValidatorSet};
pub use vote::{Vote, VoteError, VoteKind, VoteSet};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use veilux_kernel::{Block, Hash, PartyId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub jail_threshold: u64,
    pub max_round: u32,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        ConsensusConfig {
            jail_threshold: 50,
            max_round: 8,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConsensusError {
    #[error("no active validators")]
    NoValidators,
    #[error("wrong proposer: expected {expected}, got {got}")]
    WrongProposer { expected: String, got: String },
    #[error("proposer {0} is not an active validator")]
    ProposerNotValidator(String),
    #[error("parent mismatch: block parent does not match chain head")]
    ParentMismatch,
    #[error("bad height: expected {expected}, got {got}")]
    BadHeight { expected: u64, got: u64 },
    #[error("events root mismatch")]
    EventsRootMismatch,
    #[error("bad vote signature: {0}")]
    BadVoteSignature(String),
    #[error(transparent)]
    Vote(#[from] VoteError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
    Committed,
}

pub struct Aurora {
    pub config: ConsensusConfig,
    pub validators: ValidatorSet,
    pub me: Option<PartyId>,
    rounds: HashMap<(u64, u32), VoteSet>,
    committed: HashMap<u64, Hash>,
}

impl Aurora {
    pub fn new(config: ConsensusConfig, validators: ValidatorSet, me: Option<PartyId>) -> Self {
        Aurora {
            config,
            validators,
            me,
            rounds: HashMap::new(),
            committed: HashMap::new(),
        }
    }

    pub fn proposer_for(&self, height: u64, round: u32) -> Option<PartyId> {
        self.validators.proposer_for(height, round)
    }

    pub fn is_local_proposer(&self, height: u64, round: u32) -> bool {
        match (&self.me, self.proposer_for(height, round)) {
            (Some(me), Some(p)) => *me == p,
            _ => false,
        }
    }

    pub fn verify_proposal(
        &self,
        block: &Block,
        parent_hash: Hash,
        round: u32,
    ) -> Result<(), ConsensusError> {
        let expected = self
            .proposer_for(block.height, round)
            .ok_or(ConsensusError::NoValidators)?;
        if block.proposer != expected {
            return Err(ConsensusError::WrongProposer {
                expected: expected.0,
                got: block.proposer.0.clone(),
            });
        }
        if !self.validators.is_validator(&block.proposer) {
            return Err(ConsensusError::ProposerNotValidator(
                block.proposer.0.clone(),
            ));
        }
        if block.parent != parent_hash {
            return Err(ConsensusError::ParentMismatch);
        }
        if block.compute_events_root() != block.events_root {
            return Err(ConsensusError::EventsRootMismatch);
        }
        debug!(height = block.height, proposer = %block.proposer, "proposal verified");
        Ok(())
    }

    pub fn add_vote(&mut self, vote: &Vote) -> Result<CommitOutcome, ConsensusError> {
        let v = self
            .validators
            .get(&vote.voter)
            .ok_or_else(|| ConsensusError::Vote(VoteError::NotValidator(vote.voter.0.clone())))?;
        if vote.signature.is_empty() {
            return Err(ConsensusError::BadVoteSignature(
                "missing vote signature".into(),
            ));
        }
        veilux_veil::verify_bytes(&v.public_key, &vote.signing_bytes(), &vote.signature)
            .map_err(|e| ConsensusError::BadVoteSignature(e.to_string()))?;
        self.tally_vote(vote)
    }

    pub fn add_local_vote(&mut self, vote: &Vote) -> Result<CommitOutcome, ConsensusError> {
        match &self.me {
            Some(me) if *me == vote.voter => {}
            _ => {
                return Err(ConsensusError::BadVoteSignature(
                    "add_local_vote called for a non-local voter".into(),
                ))
            }
        }
        self.tally_vote(vote)
    }

    fn tally_vote(&mut self, vote: &Vote) -> Result<CommitOutcome, ConsensusError> {
        if self.committed.contains_key(&vote.height) {
            return Ok(CommitOutcome::AlreadyCommitted);
        }
        let vset = &self.validators;
        let quorum = vset.quorum_threshold();
        let entry = self.rounds.entry((vote.height, vote.round)).or_default();
        let is_new = entry.add(vote, vset)?;
        if !is_new {
            return Ok(CommitOutcome::Pending);
        }

        if vote.kind == VoteKind::Precommit {
            let power = entry.precommit_power(&vote.block_hash, vset);
            if power >= quorum {
                self.committed.insert(vote.height, vote.block_hash);
                let committers = entry.precommitters(&vote.block_hash);
                info!(
                    height = vote.height,
                    block = %vote.block_hash,
                    power,
                    quorum,
                    "block committed by BFT quorum"
                );
                return Ok(CommitOutcome::Committed {
                    height: vote.height,
                    block_hash: vote.block_hash,
                    committers,
                    power,
                });
            }
        }
        Ok(CommitOutcome::Pending)
    }

    pub fn has_prevote_quorum(&self, height: u64, round: u32, block: &Hash) -> bool {
        self.rounds
            .get(&(height, round))
            .map(|vs| {
                vs.prevote_power(block, &self.validators) >= self.validators.quorum_threshold()
            })
            .unwrap_or(false)
    }

    pub fn is_committed(&self, height: u64) -> Option<Hash> {
        self.committed.get(&height).copied()
    }

    pub fn note_missed_proposer(&mut self, height: u64, round: u32) {
        if let Some(p) = self.proposer_for(height, round) {
            warn!(height, %p, "proposer missed slot");
            self.validators
                .record_missed(&p, self.config.jail_threshold);
        }
    }

    pub fn prune_below(&mut self, height: u64) {
        self.rounds.retain(|&(h, _), _| h >= height);
        self.committed.retain(|&h, _| h >= height);
    }
}

#[derive(Debug, Clone)]
pub enum CommitOutcome {
    Pending,
    AlreadyCommitted,
    Committed {
        height: u64,
        block_hash: Hash,
        committers: Vec<PartyId>,
        power: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_veil::PartyIdentity;

    fn ident(name: &str) -> PartyIdentity {
        let mut seed = [0u8; 32];
        let b = name.as_bytes();
        seed[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
        PartyIdentity::from_seed(name, &seed)
    }

    fn engine() -> Aurora {
        let mut vs = ValidatorSet::new();
        for i in 1..=4u8 {
            let name = format!("v{i}");
            let id = ident(&name);
            vs.add(Validator::new(
                PartyId::new(name),
                id.public_key().to_vec(),
                100,
            ));
        }
        Aurora::new(ConsensusConfig::default(), vs, Some(PartyId::new("v1")))
    }

    fn signed(voter: &str, height: u64, round: u32, block: Hash, kind: VoteKind) -> Vote {
        let mut v = Vote {
            height,
            round,
            block_hash: block,
            voter: PartyId::new(voter),
            kind,
            signature: vec![],
        };
        v.signature = ident(voter).sign_bytes(&v.signing_bytes());
        v
    }

    fn precommit(voter: &str, height: u64, block: Hash) -> Vote {
        signed(voter, height, 0, block, VoteKind::Precommit)
    }

    fn vote_at(voter: &str, height: u64, round: u32, block: Hash, kind: VoteKind) -> Vote {
        signed(voter, height, round, block, kind)
    }

    #[test]
    fn rejects_unsigned_network_vote() {
        let mut e = engine();
        let b = Hash::digest(b"blk");
        let unsigned = Vote {
            height: 1,
            round: 0,
            block_hash: b,
            voter: PartyId::new("v2"),
            kind: VoteKind::Precommit,
            signature: vec![],
        };
        assert!(
            matches!(
                e.add_vote(&unsigned),
                Err(ConsensusError::BadVoteSignature(_))
            ),
            "a network vote with an empty signature must be rejected, not counted toward quorum"
        );
    }

    #[test]
    fn rejects_forged_signature_for_another_validator() {
        let mut e = engine();
        let b = Hash::digest(b"blk");
        let mut v = Vote {
            height: 1,
            round: 0,
            block_hash: b,
            voter: PartyId::new("v2"),
            kind: VoteKind::Precommit,
            signature: vec![],
        };
        v.signature = ident("attacker").sign_bytes(&v.signing_bytes());
        assert!(
            matches!(e.add_vote(&v), Err(ConsensusError::BadVoteSignature(_))),
            "a vote signed by the wrong key must be rejected"
        );
    }

    #[test]
    fn commits_at_two_thirds() {
        let mut e = engine();
        let b = Hash::digest(b"blk");
        assert!(matches!(
            e.add_vote(&precommit("v1", 1, b)).unwrap(),
            CommitOutcome::Pending
        ));
        assert!(matches!(
            e.add_vote(&precommit("v2", 1, b)).unwrap(),
            CommitOutcome::Pending
        ));
        let out = e.add_vote(&precommit("v3", 1, b)).unwrap();
        assert!(matches!(out, CommitOutcome::Committed { .. }));
        assert_eq!(e.is_committed(1), Some(b));
    }

    #[test]
    fn proposer_rotation_is_deterministic() {
        let e = engine();
        assert_eq!(e.proposer_for(0, 0), e.proposer_for(0, 0));
    }

    #[test]
    fn voting_across_views_is_not_equivocation() {
        let mut e = engine();
        let view0 = Hash::digest(b"view-0-block");
        let view1 = Hash::digest(b"view-1-block");

        e.add_vote(&vote_at("v1", 1, 0, view0, VoteKind::Prevote))
            .unwrap();
        e.add_vote(&vote_at("v2", 1, 0, view0, VoteKind::Prevote))
            .unwrap();

        for v in ["v1", "v2", "v3"] {
            e.add_vote(&vote_at(v, 1, 1, view1, VoteKind::Prevote))
                .expect("higher-round prevote must be accepted");
        }
        assert!(e.has_prevote_quorum(1, 1, &view1));
        assert!(!e.has_prevote_quorum(1, 0, &view1));

        for v in ["v1", "v2", "v3"] {
            let out = e
                .add_vote(&vote_at(v, 1, 1, view1, VoteKind::Precommit))
                .unwrap();
            if let CommitOutcome::Committed { block_hash, .. } = out {
                assert_eq!(block_hash, view1);
            }
        }
        assert_eq!(e.is_committed(1), Some(view1));
    }
}
