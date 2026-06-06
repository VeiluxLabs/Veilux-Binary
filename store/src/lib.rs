use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use tracing::{debug, info};
use veilux_kernel::{Block, Hash, SignedCommand, StateTree};

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
    mempool_path: PathBuf,
}

impl Store {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let blocks_path = dir.join("blocks.jsonl");
        let state_path = dir.join("state.json");
        let mempool_path = dir.join("mempool.jsonl");
        if !blocks_path.exists() {
            File::create(&blocks_path)?;
        }
        info!(dir = %dir.display(), "store opened");
        Ok(Store {
            dir,
            blocks_path,
            state_path,
            mempool_path,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn atomic_write(&self, final_path: &Path, tmp: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        {
            let mut f = File::create(tmp)?;
            f.write_all(bytes)?;
            f.flush()?;
            f.sync_all()?;
        }
        fs::rename(tmp, final_path)?;
        if let Ok(dirf) = File::open(&self.dir) {
            let _ = dirf.sync_all();
        }
        Ok(())
    }

    pub fn append_block(&self, block: &Block) -> Result<(), StoreError> {
        let mut f = OpenOptions::new().append(true).open(&self.blocks_path)?;
        let line = serde_json::to_string(block)?;
        writeln!(f, "{line}")?;
        f.flush()?;
        f.sync_all()?;
        debug!(height = block.height, "block appended to log (fsync'd)");
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
        self.atomic_write(&self.state_path, &tmp, &bytes)?;
        debug!(entries = state.len(), "state snapshot saved (fsync'd)");
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

    pub fn save_private_state(&self, state: &StateTree) -> Result<(), StoreError> {
        let path = self.dir.join("private_state.json");
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(state)?;
        self.atomic_write(&path, &tmp, &bytes)?;
        Ok(())
    }

    pub fn load_private_state(&self) -> Result<Option<StateTree>, StoreError> {
        let path = self.dir.join("private_state.json");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let state: StateTree = serde_json::from_slice(&bytes)?;
        Ok(Some(state))
    }

    pub fn save_private_commitments(&self, commitments: &[Hash]) -> Result<(), StoreError> {
        let path = self.dir.join("private_commitments.json");
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(commitments)?;
        self.atomic_write(&path, &tmp, &bytes)?;
        Ok(())
    }

    pub fn load_private_commitments(&self) -> Result<Vec<Hash>, StoreError> {
        let path = self.dir.join("private_commitments.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&path)?;
        let commitments: Vec<Hash> = serde_json::from_slice(&bytes)?;
        Ok(commitments)
    }

    pub fn append_pending(&self, signed: &SignedCommand) -> Result<(), StoreError> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.mempool_path)?;
        let line = serde_json::to_string(signed)?;
        writeln!(f, "{line}")?;
        f.flush()?;
        f.sync_all()?;
        debug!(party = %signed.command.submitter.0, nonce = signed.command.nonce, "pending tx persisted (fsync'd)");
        Ok(())
    }

    pub fn load_pending(&self) -> Result<Vec<SignedCommand>, StoreError> {
        if !self.mempool_path.exists() {
            return Ok(Vec::new());
        }
        let f = File::open(&self.mempool_path)?;
        let reader = BufReader::new(f);
        let mut pending = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(signed) = serde_json::from_str::<SignedCommand>(&line) {
                pending.push(signed);
            }
        }
        debug!(count = pending.len(), "pending txs loaded from mempool log");
        Ok(pending)
    }

    pub fn rewrite_pending(&self, pending: &[SignedCommand]) -> Result<(), StoreError> {
        let tmp = self.mempool_path.with_extension("jsonl.tmp");
        {
            let mut f = File::create(&tmp)?;
            for signed in pending {
                let line = serde_json::to_string(signed)?;
                writeln!(f, "{line}")?;
            }
            f.flush()?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.mempool_path)?;
        if let Ok(dirf) = File::open(&self.dir) {
            let _ = dirf.sync_all();
        }
        debug!(count = pending.len(), "mempool log rewritten (fsync'd)");
        Ok(())
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

    #[test]
    fn durable_writes_leave_no_tmp_files() {
        let dir = tmp_dir("durable");
        let store = Store::open(&dir).unwrap();
        let mut st = StateTree::new();
        st.put("a", vec![9, 9, 9]);
        store.save_state(&st).unwrap();
        store.save_private_state(&st).unwrap();
        store
            .save_private_commitments(&[Hash([1u8; 32]), Hash([2u8; 32])])
            .unwrap();

        let leftover: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(
            leftover.is_empty(),
            "atomic rename must leave no .tmp files behind"
        );

        let reloaded = store.load_state().unwrap().unwrap();
        assert_eq!(reloaded.get("a"), Some(&vec![9, 9, 9]));
        assert_eq!(store.load_private_commitments().unwrap().len(), 2);
        fs::remove_dir_all(&dir).ok();
    }
}
