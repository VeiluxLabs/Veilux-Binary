use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

const BLOB_PREFIX: &str = "storage/blob/";
const PIN_PREFIX: &str = "storage/pin/";
pub const MAX_BLOB_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StorageCommand {
    Put { key: String, data: Vec<u8> },
    Op(StorageOp),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StorageOp {
    Pin { cid: Hash },
    Unpin { cid: Hash },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageEvent {
    Stored {
        cid: Hash,
        size: u64,
        key: String,
    },
    StoredPrivate {
        cid: Hash,
        size: u64,
        key: String,
        data: Vec<u8>,
    },
    Pinned {
        cid: Hash,
        refcount: u64,
    },
    Unpinned {
        cid: Hash,
        refcount: u64,
        removed: bool,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PinRecord {
    refcount: u64,
    size: u64,
    #[serde(default)]
    public: bool,
    owner: Option<PartyId>,
}

#[derive(Default)]
pub struct StoragePrism;

impl StoragePrism {
    pub fn new() -> Self {
        StoragePrism
    }

    fn blob_key(cid: &Hash) -> String {
        format!("{BLOB_PREFIX}{}", cid.to_hex())
    }

    fn pin_key(cid: &Hash) -> String {
        format!("{PIN_PREFIX}{}", cid.to_hex())
    }

    fn load_pin(state: &StateTree, cid: &Hash) -> PinRecord {
        state
            .get_json::<PinRecord>(&Self::pin_key(cid))
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

impl Prism for StoragePrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "storage",
            description: "Content-addressed blob storage with reference-counted pinning",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: StorageCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            StorageCommand::Put { key, data } => {
                if data.len() > MAX_BLOB_SIZE {
                    return Err(PrismError::LimitExceeded(format!(
                        "blob of {} bytes exceeds max {}",
                        data.len(),
                        MAX_BLOB_SIZE
                    )));
                }
                let cid = Hash::digest(&data);
                let size = data.len() as u64;
                let public = matches!(command.visibility, Visibility::Public);

                if public && !state.contains(&Self::blob_key(&cid)) {
                    state.put(Self::blob_key(&cid), data.clone());
                }

                let mut pin = Self::load_pin(state, &cid);
                pin.refcount += 1;
                pin.size = size;
                pin.public = public;
                pin.owner = Some(command.submitter.clone());
                state
                    .put_json(Self::pin_key(&cid), &pin)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let payload = if public {
                    serde_json::to_vec(&StorageEvent::Stored { cid, size, key }).unwrap_or_default()
                } else {
                    serde_json::to_vec(&StorageEvent::StoredPrivate {
                        cid,
                        size,
                        key,
                        data,
                    })
                    .unwrap_or_default()
                };
                let event = Event {
                    source_command: command.id(),
                    prism: "storage".into(),
                    visibility: command.visibility.clone(),
                    payload,
                };
                Ok(PrismOutput::single(event, 100 + size * 2))
            }

            StorageCommand::Op(StorageOp::Pin { cid }) => {
                if !state.contains(&Self::blob_key(&cid)) {
                    return Err(PrismError::NotFound(format!("blob {}", cid.to_hex())));
                }
                let mut pin = Self::load_pin(state, &cid);
                pin.refcount += 1;
                state
                    .put_json(Self::pin_key(&cid), &pin)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let event = Event {
                    source_command: command.id(),
                    prism: "storage".into(),
                    visibility: command.visibility.clone(),
                    payload: serde_json::to_vec(&StorageEvent::Pinned {
                        cid,
                        refcount: pin.refcount,
                    })
                    .unwrap_or_default(),
                };
                Ok(PrismOutput::single(event, 200))
            }

            StorageCommand::Op(StorageOp::Unpin { cid }) => {
                let mut pin = Self::load_pin(state, &cid);
                if pin.refcount == 0 {
                    return Err(PrismError::NotFound(format!("no pin for {}", cid.to_hex())));
                }
                pin.refcount -= 1;
                let removed = pin.refcount == 0;
                if removed {
                    state.remove(&Self::blob_key(&cid));
                    state.remove(&Self::pin_key(&cid));
                } else {
                    state
                        .put_json(Self::pin_key(&cid), &pin)
                        .map_err(|e| PrismError::Internal(e.to_string()))?;
                }

                let event = Event {
                    source_command: command.id(),
                    prism: "storage".into(),
                    visibility: command.visibility.clone(),
                    payload: serde_json::to_vec(&StorageEvent::Unpinned {
                        cid,
                        refcount: pin.refcount,
                        removed,
                    })
                    .unwrap_or_default(),
                };
                Ok(PrismOutput::single(event, 150))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<StorageCommand>(&command.payload) {
            Ok(StorageCommand::Put { data, .. }) => 100 + data.len() as u64 * 2,
            Ok(StorageCommand::Op(_)) => 200,
            Err(_) => 1_000,
        }
    }
}

pub fn put_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    key: &str,
    data: Vec<u8>,
) -> Command {
    let payload = serde_json::to_vec(&StorageCommand::Put {
        key: key.to_string(),
        data,
    })
    .unwrap_or_default();
    Command {
        prism: "storage".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn read_blob(state: &StateTree, cid: &Hash) -> Option<Vec<u8>> {
    state.get(&StoragePrism::blob_key(cid)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_is_content_addressed_and_dedups() {
        let prism = StoragePrism::new();
        let mut state = StateTree::new();

        let c1 = put_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "a.txt",
            b"hello".to_vec(),
        );
        let c2 = put_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            "b.txt",
            b"hello".to_vec(),
        );

        let e1 = prism.handle(&c1, &mut state).unwrap();
        let _e2 = prism.handle(&c2, &mut state).unwrap();

        let cid = match serde_json::from_slice::<StorageEvent>(&e1.events[0].payload).unwrap() {
            StorageEvent::Stored { cid, .. } => cid,
            _ => unreachable!(),
        };
        let blobs = state.iter_prefix(BLOB_PREFIX).count();
        assert_eq!(blobs, 1);
        assert_eq!(read_blob(&state, &cid).unwrap(), b"hello");
    }

    #[test]
    fn unpin_gc_removes_blob() {
        let prism = StoragePrism::new();
        let mut state = StateTree::new();
        let c = put_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "x",
            b"data".to_vec(),
        );
        let e = prism.handle(&c, &mut state).unwrap();
        let cid = match serde_json::from_slice::<StorageEvent>(&e.events[0].payload).unwrap() {
            StorageEvent::Stored { cid, .. } => cid,
            _ => unreachable!(),
        };

        let unpin = Command {
            prism: "storage".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&StorageCommand::Op(StorageOp::Unpin { cid })).unwrap(),
            nonce: 1,
        };
        prism.handle(&unpin, &mut state).unwrap();
        assert!(read_blob(&state, &cid).is_none());
    }

    #[test]
    fn private_blob_bytes_never_enter_public_state() {
        let prism = StoragePrism::new();
        let mut state = StateTree::new();
        let secret = b"top-secret-contract-bytes".to_vec();
        let cmd = put_command(
            PartyId::new("alice"),
            Visibility::Parties(vec![PartyId::new("alice"), PartyId::new("bob")]),
            0,
            "secret.bin",
            secret.clone(),
        );
        let out = prism.handle(&cmd, &mut state).unwrap();

        let cid = Hash::digest(&secret);
        assert!(
            read_blob(&state, &cid).is_none(),
            "private blob must NOT be in public state"
        );
        assert_eq!(state.iter_prefix(BLOB_PREFIX).count(), 0);

        let leaked = state
            .iter_prefix("")
            .any(|(_, v)| v.windows(secret.len()).any(|w| w == secret.as_slice()));
        assert!(!leaked, "secret bytes leaked into public state");

        match serde_json::from_slice::<StorageEvent>(&out.events[0].payload).unwrap() {
            StorageEvent::StoredPrivate {
                data, cid: ev_cid, ..
            } => {
                assert_eq!(data, secret);
                assert_eq!(ev_cid, cid);
            }
            _ => panic!("expected StoredPrivate event carrying the sealed payload"),
        }
        assert_eq!(out.events[0].visibility.stakeholders().len(), 2);
    }

    #[test]
    fn public_blob_still_readable() {
        let prism = StoragePrism::new();
        let mut state = StateTree::new();
        let cmd = put_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "pub.bin",
            b"public-data".to_vec(),
        );
        prism.handle(&cmd, &mut state).unwrap();
        let cid = Hash::digest(b"public-data");
        assert_eq!(read_blob(&state, &cid).unwrap(), b"public-data");
    }
}
