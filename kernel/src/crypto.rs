use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl Default for Hash {
    fn default() -> Self {
        Hash::ZERO
    }
}

impl Hash {
    pub const ZERO: Hash = Hash([0u8; 32]);

    pub fn digest(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn combine(left: &Hash, right: &Hash) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&left.0);
        hasher.update(&right.0);
        Hash(*hasher.finalize().as_bytes())
    }

    pub fn commit(domain: &str, parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain.as_bytes());
        hasher.update(&[0xff]);
        for p in parts {
            hasher.update(&(p.len() as u64).to_le_bytes());
            hasher.update(p);
        }
        Hash(*hasher.finalize().as_bytes())
    }

    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim_start_matches("0x");
        let bytes = hex::decode(s).ok()?;
        if bytes.len() != 32 {
            return None;
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Some(Hash(out))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}..", hex::encode(&self.0[..4]))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl From<[u8; 32]> for Hash {
    fn from(v: [u8; 32]) -> Self {
        Hash(v)
    }
}

pub fn merkle_root(leaves: &[Hash]) -> Hash {
    if leaves.is_empty() {
        return Hash::ZERO;
    }
    let mut level: Vec<Hash> = leaves.to_vec();
    while level.len() > 1 {
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        for chunk in level.chunks(2) {
            let combined = if chunk.len() == 2 {
                Hash::combine(&chunk[0], &chunk[1])
            } else {
                Hash::combine(&chunk[0], &chunk[0])
            };
            next.push(combined);
        }
        level = next;
    }
    level[0]
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MerkleStep {
    pub sibling: Hash,
    pub sibling_on_right: bool,
}

pub fn merkle_proof(leaves: &[Hash], index: usize) -> Vec<MerkleStep> {
    let mut proof = Vec::new();
    if index >= leaves.len() {
        return proof;
    }
    let mut idx = index;
    let mut level: Vec<Hash> = leaves.to_vec();
    while level.len() > 1 {
        let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
        let sibling = if sibling_idx < level.len() {
            level[sibling_idx]
        } else {
            level[idx]
        };
        proof.push(MerkleStep {
            sibling,
            sibling_on_right: idx % 2 == 0,
        });
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        for chunk in level.chunks(2) {
            let combined = if chunk.len() == 2 {
                Hash::combine(&chunk[0], &chunk[1])
            } else {
                Hash::combine(&chunk[0], &chunk[0])
            };
            next.push(combined);
        }
        level = next;
        idx /= 2;
    }
    proof
}

pub fn verify_merkle_proof(leaf: &Hash, proof: &[MerkleStep], root: &Hash) -> bool {
    let mut acc = *leaf;
    for step in proof {
        acc = if step.sibling_on_right {
            Hash::combine(&acc, &step.sibling)
        } else {
            Hash::combine(&step.sibling, &acc)
        };
    }
    &acc == root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_hex() {
        let h = Hash::digest(b"veilux");
        let s = h.to_hex();
        assert_eq!(Hash::from_hex(&s), Some(h));
    }

    #[test]
    fn merkle_empty_is_zero() {
        assert_eq!(merkle_root(&[]), Hash::ZERO);
    }

    #[test]
    fn merkle_single_leaf() {
        let h = Hash::digest(b"a");
        assert_eq!(merkle_root(&[h]), h);
    }

    #[test]
    fn merkle_is_deterministic() {
        let leaves: Vec<Hash> = (0..5u8).map(|i| Hash::digest(&[i])).collect();
        assert_eq!(merkle_root(&leaves), merkle_root(&leaves));
    }
}
