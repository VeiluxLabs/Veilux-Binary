use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use tracing::{debug, info};
use veilux_kernel::{Block, StateTree};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("corrupt block log at line {0}")]
    Corrupt(usize),
}

pub struct Store {
    dir: PathBuf,
    blocks_path: PathBuf,
    state_path: PathBuf,
}

impl Store {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let blocks_path = dir.join("blocks.jsonl");
        let state_path = dir.join("state.json");
        if !blocks_path.exists() {
            File::create(&blocks_path)?;
        }
        info!(dir = %dir.display(), "store opened");
        Ok(Store {
            dir,
            blocks_path,
            state_path,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append_block(&self, block: &Block) -> Result<(), StoreError> {
        let mut f = OpenOptions::new().append(true).open(&self.blocks_path)?;
        let line = serde_json::to_string(block)?;
        writeln!(f, "{line}")?;
        debug!(height = block.height, "block appended to log");
        Ok(())
    }

    pub fn load_blocks(&self) -> Result<Vec<Block>, StoreError> {
        let f = File::open(&self.blocks_path)?;
        let reader = BufReader::new(f);
        let mut blocks = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let block: Block =
                serde_json::from_str(&line).map_err(|_| StoreError::Corrupt(i + 1))?;
            blocks.push(block);
        }
        debug!(count = blocks.len(), "blocks loaded from log");
        Ok(blocks)
    }

    pub fn block_count(&self) -> Result<usize, StoreError> {
        let f = File::open(&self.blocks_path)?;
        let reader = BufReader::new(f);
        Ok(reader
            .lines()
            .filter(|l| l.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false))
            .count())
    }

    pub fn save_state(&self, state: &StateTree) -> Result<(), StoreError> {
        let tmp = self.state_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(state)?;
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &self.state_path)?;
        debug!(entries = state.len(), "state snapshot saved");
        Ok(())
    }

    pub fn load_state(&self) -> Result<Option<StateTree>, StoreError> {
        if !self.state_path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.state_path)?;
        let state: StateTree = serde_json::from_slice(&bytes)?;
        Ok(Some(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::PartyId;

    fn tmp_dir(name: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("veilux-store-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn append_and_reload_blocks() {
        let dir = tmp_dir("blocks");
        let store = Store::open(&dir).unwrap();
        let g = Block::genesis(PartyId::new("v1"), 100);
        store.append_block(&g).unwrap();
        let mut b1 = Block::genesis(PartyId::new("v1"), 200);
        b1.height = 1;
        store.append_block(&b1).unwrap();

        let loaded = store.load_blocks().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].height, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn state_snapshot_roundtrip() {
        let dir = tmp_dir("state");
        let store = Store::open(&dir).unwrap();
        let mut st = StateTree::new();
        st.put("k", vec![1, 2, 3]);
        store.save_state(&st).unwrap();
        let loaded = store.load_state().unwrap().unwrap();
        assert_eq!(loaded.get("k"), Some(&vec![1, 2, 3]));
        assert_eq!(loaded.root(), st.root());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_state_returns_none() {
        let dir = tmp_dir("empty");
        let store = Store::open(&dir).unwrap();
        assert!(store.load_state().unwrap().is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
