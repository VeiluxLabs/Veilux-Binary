use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::crypto::{merkle_root, Hash};

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("key not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StateTree {
    entries: BTreeMap<String, Vec<u8>>,
}

impl StateTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: impl Into<String>, value: Vec<u8>) {
        self.entries.insert(key.into(), value);
    }

    pub fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.entries.remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn put_json<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: &T,
    ) -> Result<(), StateError> {
        let bytes =
            serde_json::to_vec(value).map_err(|e| StateError::Serialization(e.to_string()))?;
        self.put(key, bytes);
        Ok(())
    }

    pub fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        key: &str,
    ) -> Result<Option<T>, StateError> {
        match self.entries.get(key) {
            Some(bytes) => {
                let v = serde_json::from_slice(bytes)
                    .map_err(|e| StateError::Serialization(e.to_string()))?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    pub fn iter_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a String, &'a Vec<u8>)> {
        self.entries
            .range(prefix.to_string()..)
            .take_while(move |(k, _)| k.starts_with(prefix))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn root(&self) -> Hash {
        if self.entries.is_empty() {
            return Hash::ZERO;
        }
        let leaves: Vec<Hash> = self
            .entries
            .iter()
            .map(|(k, v)| Hash::commit("kv", &[k.as_bytes(), v]))
            .collect();
        merkle_root(&leaves)
    }

    pub fn leaf_hash(key: &str, value: &[u8]) -> Hash {
        Hash::commit("kv", &[key.as_bytes(), value])
    }

    pub fn prove(&self, key: &str) -> Option<(Vec<u8>, Vec<crate::crypto::MerkleStep>)> {
        let value = self.entries.get(key)?.clone();
        let leaves: Vec<Hash> = self
            .entries
            .iter()
            .map(|(k, v)| Hash::commit("kv", &[k.as_bytes(), v]))
            .collect();
        let index = self.entries.keys().position(|k| k == key)?;
        Some((value, crate::crypto::merkle_proof(&leaves, index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_changes_with_writes() {
        let mut s = StateTree::new();
        let r0 = s.root();
        s.put("a", vec![1, 2, 3]);
        let r1 = s.root();
        assert_ne!(r0, r1);
    }

    #[test]
    fn root_is_order_independent() {
        let mut a = StateTree::new();
        a.put("x", vec![1]);
        a.put("y", vec![2]);

        let mut b = StateTree::new();
        b.put("y", vec![2]);
        b.put("x", vec![1]);

        assert_eq!(a.root(), b.root());
    }

    #[test]
    fn prefix_iteration() {
        let mut s = StateTree::new();
        s.put("ai/models/1", vec![1]);
        s.put("ai/models/2", vec![2]);
        s.put("storage/blob/1", vec![3]);
        let count = s.iter_prefix("ai/models/").count();
        assert_eq!(count, 2);
    }
}
