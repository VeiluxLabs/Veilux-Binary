use std::collections::HashMap;

use prism_staking::EquivocationProof;
use veilux_consensus::{Vote, VoteKind};
use veilux_kernel::{Hash, PartyId};

#[derive(Clone)]
struct SeenVote {
    block_hash: Hash,
    message: Vec<u8>,
    signature: Vec<u8>,
}

#[derive(Default)]
pub struct EquivocationWatch {
    seen: HashMap<(u64, u32, u8, String), SeenVote>,
    slashed: std::collections::HashSet<String>,
}

impl EquivocationWatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prune_below(&mut self, height: u64) {
        self.seen.retain(|(h, _, _, _), _| *h >= height);
    }

    pub fn observe(&mut self, vote: &Vote, public_key: &[u8]) -> Option<EquivocationProof> {
        if vote.signature.is_empty() {
            return None;
        }
        let kind = match vote.kind {
            VoteKind::Prevote => 0u8,
            VoteKind::Precommit => 1u8,
        };
        let key = (vote.height, vote.round, kind, vote.voter.0.clone());
        let message = vote.signing_bytes();

        if let Some(prev) = self.seen.get(&key) {
            if prev.block_hash != vote.block_hash {
                if self.slashed.contains(&vote.voter.0) {
                    return None;
                }
                self.slashed.insert(vote.voter.0.clone());
                return Some(EquivocationProof {
                    offender: PartyId::new(&vote.voter.0),
                    public_key: hex::encode(public_key),
                    message_a: hex::encode(&prev.message),
                    signature_a: hex::encode(&prev.signature),
                    message_b: hex::encode(&message),
                    signature_b: hex::encode(&vote.signature),
                });
            }
            return None;
        }

        self.seen.insert(
            key,
            SeenVote {
                block_hash: vote.block_hash,
                message,
                signature: vote.signature.clone(),
            },
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_veil::PartyIdentity;

    fn signed_vote(id: &PartyIdentity, height: u64, round: u32, block: Hash) -> Vote {
        let mut v = Vote {
            height,
            round,
            block_hash: block,
            voter: id.party().clone(),
            kind: VoteKind::Prevote,
            signature: vec![],
        };
        v.signature = id.sign_bytes(&v.signing_bytes());
        v
    }

    #[test]
    fn detects_double_signed_votes() {
        let id = PartyIdentity::from_seed("bob", &[7u8; 32]);
        let mut w = EquivocationWatch::new();
        let pk = id.public_key();

        let v1 = signed_vote(&id, 5, 0, Hash::digest(b"A"));
        assert!(w.observe(&v1, &pk).is_none());

        let v2 = signed_vote(&id, 5, 0, Hash::digest(b"B"));
        let proof = w.observe(&v2, &pk).expect("equivocation detected");
        assert_eq!(proof.offender, PartyId::new("bob"));
        assert_ne!(proof.message_a, proof.message_b);
    }

    #[test]
    fn honest_repeat_vote_is_not_equivocation() {
        let id = PartyIdentity::from_seed("alice", &[1u8; 32]);
        let mut w = EquivocationWatch::new();
        let pk = id.public_key();
        let block = Hash::digest(b"A");
        let v1 = signed_vote(&id, 5, 0, block);
        let v2 = signed_vote(&id, 5, 0, block);
        assert!(w.observe(&v1, &pk).is_none());
        assert!(w.observe(&v2, &pk).is_none());
    }

    #[test]
    fn only_reports_once_per_offender() {
        let id = PartyIdentity::from_seed("bob", &[7u8; 32]);
        let mut w = EquivocationWatch::new();
        let pk = id.public_key();
        w.observe(&signed_vote(&id, 5, 0, Hash::digest(b"A")), &pk);
        assert!(w
            .observe(&signed_vote(&id, 5, 0, Hash::digest(b"B")), &pk)
            .is_some());
        assert!(w
            .observe(&signed_vote(&id, 6, 0, Hash::digest(b"C")), &pk)
            .is_none());
        let id2 = PartyIdentity::from_seed("bob", &[7u8; 32]);
        w.observe(&signed_vote(&id2, 6, 0, Hash::digest(b"D")), &pk);
        assert!(w
            .observe(&signed_vote(&id2, 6, 0, Hash::digest(b"E")), &pk)
            .is_none());
    }
}
