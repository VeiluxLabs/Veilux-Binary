use serde::{Deserialize, Serialize};

use veilux_kernel::{verify_merkle_proof, Hash, MerkleStep, PartyId, StateTree};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Header {
    pub height: u64,
    pub parent: Hash,
    pub events_root: Hash,
    pub state_root: Hash,
    pub commands_root: Hash,
    pub timestamp: u64,
    pub proposer: PartyId,
}

impl Header {
    pub fn hash(&self) -> Hash {
        Hash::commit(
            "block",
            &[
                &self.height.to_le_bytes(),
                self.parent.as_bytes(),
                self.events_root.as_bytes(),
                self.state_root.as_bytes(),
                self.commands_root.as_bytes(),
                &self.timestamp.to_le_bytes(),
                self.proposer.0.as_bytes(),
            ],
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LightError {
    #[error("header chain broken at height {0}: parent hash mismatch")]
    BrokenChain(u64),
    #[error("non-monotonic height at index {0}")]
    BadHeight(usize),
    #[error("state proof failed: value not committed under state_root")]
    BadStateProof,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateProof {
    pub key: String,
    pub value: Vec<u8>,
    pub proof: Vec<MerkleStep>,
    pub state_root: Hash,
}

pub struct LightClient {
    trusted: Header,
}

impl LightClient {
    pub fn new(trusted_genesis: Header) -> Self {
        LightClient {
            trusted: trusted_genesis,
        }
    }

    pub fn head(&self) -> &Header {
        &self.trusted
    }

    pub fn apply_headers(&mut self, headers: &[Header]) -> Result<(), LightError> {
        for (i, h) in headers.iter().enumerate() {
            if h.height != self.trusted.height + 1 {
                return Err(LightError::BadHeight(i));
            }
            if h.parent != self.trusted.hash() {
                return Err(LightError::BrokenChain(h.height));
            }
            self.trusted = h.clone();
        }
        Ok(())
    }

    pub fn verify_state(&self, proof: &StateProof) -> Result<(), LightError> {
        if proof.state_root != self.trusted.state_root {
            return Err(LightError::BadStateProof);
        }
        let leaf = StateTree::leaf_hash(&proof.key, &proof.value);
        if verify_merkle_proof(&leaf, &proof.proof, &proof.state_root) {
            Ok(())
        } else {
            Err(LightError::BadStateProof)
        }
    }
}

pub fn verify_state_against_root(proof: &StateProof) -> bool {
    let leaf = StateTree::leaf_hash(&proof.key, &proof.value);
    verify_merkle_proof(&leaf, &proof.proof, &proof.state_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(height: u64, parent: Hash, state_root: Hash) -> Header {
        Header {
            height,
            parent,
            events_root: Hash::ZERO,
            state_root,
            commands_root: Hash::ZERO,
            timestamp: height * 10,
            proposer: PartyId::new("v0"),
        }
    }

    #[test]
    fn follows_valid_header_chain() {
        let genesis = hdr(0, Hash::ZERO, Hash::ZERO);
        let mut lc = LightClient::new(genesis.clone());
        let h1 = hdr(1, genesis.hash(), Hash::digest(b"s1"));
        let h2 = hdr(2, h1.hash(), Hash::digest(b"s2"));
        lc.apply_headers(&[h1, h2.clone()]).unwrap();
        assert_eq!(lc.head().height, 2);
        assert_eq!(lc.head().hash(), h2.hash());
    }

    #[test]
    fn rejects_forged_parent() {
        let genesis = hdr(0, Hash::ZERO, Hash::ZERO);
        let mut lc = LightClient::new(genesis);
        let forged = hdr(1, Hash::digest(b"not-the-genesis"), Hash::ZERO);
        assert!(matches!(
            lc.apply_headers(&[forged]),
            Err(LightError::BrokenChain(1))
        ));
    }

    #[test]
    fn verifies_real_state_inclusion_proof() {
        let mut state = StateTree::new();
        state.put("token/bal/x", vec![1, 2, 3]);
        state.put("token/bal/y", vec![4, 5, 6]);
        state.put("name/rec/alice.veil", vec![7, 8]);
        let root = state.root();

        let (value, proof) = state.prove("token/bal/y").unwrap();
        let genesis = hdr(0, Hash::ZERO, root);
        let lc = LightClient::new(genesis);
        let sp = StateProof {
            key: "token/bal/y".into(),
            value,
            proof,
            state_root: root,
        };
        assert!(lc.verify_state(&sp).is_ok());
    }

    #[test]
    fn rejects_tampered_state_value() {
        let mut state = StateTree::new();
        state.put("a", vec![1]);
        state.put("b", vec![2]);
        state.put("c", vec![3]);
        let root = state.root();
        let (_v, proof) = state.prove("b").unwrap();
        let sp = StateProof {
            key: "b".into(),
            value: vec![99],
            proof,
            state_root: root,
        };
        assert!(!verify_state_against_root(&sp));
    }
}
