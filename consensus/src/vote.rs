use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use veilux_kernel::{Hash, PartyId};

use crate::validator::ValidatorSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteKind {
    Prevote,
    Precommit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vote {
    pub height: u64,
    pub round: u32,
    pub block_hash: Hash,
    pub voter: PartyId,
    pub kind: VoteKind,
    pub signature: Vec<u8>,
}

impl Vote {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(80);
        v.extend_from_slice(b"veilux/vote/v1");
        v.push(0xff);
        v.extend_from_slice(&self.height.to_le_bytes());
        v.extend_from_slice(&self.round.to_le_bytes());
        v.extend_from_slice(self.block_hash.as_bytes());
        v.push(match self.kind {
            VoteKind::Prevote => 0,
            VoteKind::Precommit => 1,
        });
        v.extend_from_slice(self.voter.0.as_bytes());
        v
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoteError {
    #[error("voter {0} is not an active validator")]
    NotValidator(String),
    #[error("equivocation: {0} voted for two different blocks at the same height/round")]
    Equivocation(String),
}

#[derive(Default)]
pub struct VoteSet {
    prevotes: HashMap<Hash, Vec<PartyId>>,
    precommits: HashMap<Hash, Vec<PartyId>>,
    prevote_seen: HashMap<PartyId, Hash>,
    precommit_seen: HashMap<PartyId, Hash>,
}

impl VoteSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, vote: &Vote, vset: &ValidatorSet) -> Result<bool, VoteError> {
        if !vset.is_validator(&vote.voter) {
            return Err(VoteError::NotValidator(vote.voter.0.clone()));
        }
        let (seen, tally) = match vote.kind {
            VoteKind::Prevote => (&mut self.prevote_seen, &mut self.prevotes),
            VoteKind::Precommit => (&mut self.precommit_seen, &mut self.precommits),
        };
        if let Some(prev) = seen.get(&vote.voter) {
            if *prev != vote.block_hash {
                return Err(VoteError::Equivocation(vote.voter.0.clone()));
            }
            return Ok(false);
        }
        seen.insert(vote.voter.clone(), vote.block_hash);
        tally
            .entry(vote.block_hash)
            .or_default()
            .push(vote.voter.clone());
        Ok(true)
    }

    pub fn prevote_power(&self, block: &Hash, vset: &ValidatorSet) -> u64 {
        Self::power(&self.prevotes, block, vset)
    }

    pub fn precommit_power(&self, block: &Hash, vset: &ValidatorSet) -> u64 {
        Self::power(&self.precommits, block, vset)
    }

    fn power(tally: &HashMap<Hash, Vec<PartyId>>, block: &Hash, vset: &ValidatorSet) -> u64 {
        tally
            .get(block)
            .map(|voters| {
                voters
                    .iter()
                    .filter_map(|p| vset.get(p))
                    .map(|v| v.stake)
                    .sum()
            })
            .unwrap_or(0)
    }

    pub fn precommitters(&self, block: &Hash) -> Vec<PartyId> {
        self.precommits.get(block).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator::Validator;

    fn vset() -> ValidatorSet {
        let mut s = ValidatorSet::new();
        for i in 1..=4u8 {
            s.add(Validator::new(
                PartyId::new(format!("v{i}")),
                vec![i; 32],
                100,
            ));
        }
        s
    }

    fn vote(voter: &str, block: Hash, kind: VoteKind) -> Vote {
        Vote {
            height: 1,
            round: 0,
            block_hash: block,
            voter: PartyId::new(voter),
            kind,
            signature: vec![],
        }
    }

    #[test]
    fn tally_reaches_quorum() {
        let s = vset();
        let mut vs = VoteSet::new();
        let b = Hash::digest(b"block");
        for v in ["v1", "v2", "v3"] {
            vs.add(&vote(v, b, VoteKind::Precommit), &s).unwrap();
        }
        assert!(vs.precommit_power(&b, &s) >= s.quorum_threshold());
    }

    #[test]
    fn equivocation_detected() {
        let s = vset();
        let mut vs = VoteSet::new();
        vs.add(&vote("v1", Hash::digest(b"a"), VoteKind::Prevote), &s)
            .unwrap();
        let err = vs.add(&vote("v1", Hash::digest(b"b"), VoteKind::Prevote), &s);
        assert!(matches!(err, Err(VoteError::Equivocation(_))));
    }

    #[test]
    fn non_validator_rejected() {
        let s = vset();
        let mut vs = VoteSet::new();
        let err = vs.add(&vote("ghost", Hash::digest(b"a"), VoteKind::Prevote), &s);
        assert!(matches!(err, Err(VoteError::NotValidator(_))));
    }
}
