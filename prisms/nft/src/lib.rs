use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

const COLL_PREFIX: &str = "nft/coll/";
const OWNER_PREFIX: &str = "nft/owner/";
const META_PREFIX: &str = "nft/token/";
const APPROVE_PREFIX: &str = "nft/approve/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Collection {
    pub collection_id: Hash,
    pub name: String,
    pub symbol: String,
    pub creator: PartyId,
    pub minted: u64,
    pub max_supply: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NftToken {
    pub collection_id: Hash,
    pub token_index: u64,
    pub metadata_uri: String,
    pub content_hash: Hash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NftCommand {
    CreateCollection {
        name: String,
        symbol: String,
        max_supply: Option<u64>,
    },
    Mint {
        collection_id: Hash,
        to: PartyId,
        metadata_uri: String,
        content_hash: Hash,
    },
    Transfer {
        collection_id: Hash,
        token_index: u64,
        to: PartyId,
    },
    Approve {
        collection_id: Hash,
        token_index: u64,
        spender: PartyId,
    },
    Burn {
        collection_id: Hash,
        token_index: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NftEvent {
    CollectionCreated {
        collection_id: Hash,
        symbol: String,
    },
    Minted {
        collection_id: Hash,
        token_index: u64,
        to: PartyId,
    },
    Transfer {
        collection_id: Hash,
        token_index: u64,
        from: PartyId,
        to: PartyId,
    },
    Approval {
        collection_id: Hash,
        token_index: u64,
        owner: PartyId,
        spender: PartyId,
    },
    Burned {
        collection_id: Hash,
        token_index: u64,
    },
}

#[derive(Default)]
pub struct NftPrism;

impl NftPrism {
    pub fn new() -> Self {
        NftPrism
    }

    fn coll_key(id: &Hash) -> String {
        format!("{COLL_PREFIX}{}", id.to_hex())
    }

    fn owner_key(id: &Hash, idx: u64) -> String {
        format!("{OWNER_PREFIX}{}/{}", id.to_hex(), idx)
    }

    fn meta_key(id: &Hash, idx: u64) -> String {
        format!("{META_PREFIX}{}/{}", id.to_hex(), idx)
    }

    fn approve_key(id: &Hash, idx: u64) -> String {
        format!("{APPROVE_PREFIX}{}/{}", id.to_hex(), idx)
    }

    fn load_collection(state: &StateTree, id: &Hash) -> Result<Collection, PrismError> {
        state
            .get_json::<Collection>(&Self::coll_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("collection {}", id.to_hex())))
    }

    fn owner_of(state: &StateTree, id: &Hash, idx: u64) -> Option<PartyId> {
        state.get_json(&Self::owner_key(id, idx)).ok().flatten()
    }

    fn event(cmd: &Command, payload: NftEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "nft".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for NftPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "nft",
            description: "Non-fungible tokens: collections, mint, transfer, approve, burn",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: NftCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            NftCommand::CreateCollection {
                name,
                symbol,
                max_supply,
            } => {
                let collection_id = Hash::commit(
                    "nft/coll-id",
                    &[
                        command.submitter.0.as_bytes(),
                        symbol.as_bytes(),
                        name.as_bytes(),
                    ],
                );
                if state.contains(&Self::coll_key(&collection_id)) {
                    return Err(PrismError::InvalidPayload("collection exists".into()));
                }
                let coll = Collection {
                    collection_id,
                    name,
                    symbol: symbol.clone(),
                    creator: command.submitter.clone(),
                    minted: 0,
                    max_supply,
                };
                state
                    .put_json(Self::coll_key(&collection_id), &coll)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NftEvent::CollectionCreated {
                            collection_id,
                            symbol,
                        },
                    ),
                    5_000,
                ))
            }

            NftCommand::Mint {
                collection_id,
                to,
                metadata_uri,
                content_hash,
            } => {
                let mut coll = Self::load_collection(state, &collection_id)?;
                if coll.creator != command.submitter {
                    return Err(PrismError::Unauthorized("only creator can mint".into()));
                }
                if let Some(max) = coll.max_supply {
                    if coll.minted >= max {
                        return Err(PrismError::LimitExceeded("max supply reached".into()));
                    }
                }
                let token_index = coll.minted;
                let token = NftToken {
                    collection_id,
                    token_index,
                    metadata_uri,
                    content_hash,
                };
                state
                    .put_json(Self::meta_key(&collection_id, token_index), &token)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                state
                    .put_json(Self::owner_key(&collection_id, token_index), &to)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                coll.minted += 1;
                state
                    .put_json(Self::coll_key(&collection_id), &coll)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NftEvent::Minted {
                            collection_id,
                            token_index,
                            to,
                        },
                    ),
                    3_000,
                ))
            }

            NftCommand::Transfer {
                collection_id,
                token_index,
                to,
            } => {
                let owner = Self::owner_of(state, &collection_id, token_index)
                    .ok_or_else(|| PrismError::NotFound("token".into()))?;
                let approved = state
                    .get_json::<PartyId>(&Self::approve_key(&collection_id, token_index))
                    .ok()
                    .flatten();
                let caller = &command.submitter;
                if &owner != caller && approved.as_ref() != Some(caller) {
                    return Err(PrismError::Unauthorized("not owner or approved".into()));
                }
                state
                    .put_json(Self::owner_key(&collection_id, token_index), &to)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                state.remove(&Self::approve_key(&collection_id, token_index));

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NftEvent::Transfer {
                            collection_id,
                            token_index,
                            from: owner,
                            to,
                        },
                    ),
                    1_200,
                ))
            }

            NftCommand::Approve {
                collection_id,
                token_index,
                spender,
            } => {
                let owner = Self::owner_of(state, &collection_id, token_index)
                    .ok_or_else(|| PrismError::NotFound("token".into()))?;
                if owner != command.submitter {
                    return Err(PrismError::Unauthorized("only owner can approve".into()));
                }
                state
                    .put_json(Self::approve_key(&collection_id, token_index), &spender)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NftEvent::Approval {
                            collection_id,
                            token_index,
                            owner,
                            spender,
                        },
                    ),
                    800,
                ))
            }

            NftCommand::Burn {
                collection_id,
                token_index,
            } => {
                let owner = Self::owner_of(state, &collection_id, token_index)
                    .ok_or_else(|| PrismError::NotFound("token".into()))?;
                if owner != command.submitter {
                    return Err(PrismError::Unauthorized("only owner can burn".into()));
                }
                state.remove(&Self::owner_key(&collection_id, token_index));
                state.remove(&Self::meta_key(&collection_id, token_index));
                state.remove(&Self::approve_key(&collection_id, token_index));

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NftEvent::Burned {
                            collection_id,
                            token_index,
                        },
                    ),
                    1_000,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<NftCommand>(&command.payload) {
            Ok(NftCommand::CreateCollection { .. }) => 5_000,
            Ok(NftCommand::Mint { .. }) => 3_000,
            Ok(_) => 1_200,
            Err(_) => 1_000,
        }
    }
}

pub fn owner_of(state: &StateTree, collection_id: &Hash, token_index: u64) -> Option<PartyId> {
    NftPrism::owner_of(state, collection_id, token_index)
}

pub fn create_collection_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    symbol: &str,
    max_supply: Option<u64>,
) -> Command {
    let payload = serde_json::to_vec(&NftCommand::CreateCollection {
        name: name.to_string(),
        symbol: symbol.to_string(),
        max_supply,
    })
    .unwrap_or_default();
    Command {
        prism: "nft".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coll_id(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<NftEvent>(&out.events[0].payload).unwrap() {
            NftEvent::CollectionCreated { collection_id, .. } => collection_id,
            _ => panic!("expected CollectionCreated"),
        }
    }

    #[test]
    fn mint_and_transfer() {
        let p = NftPrism::new();
        let mut s = StateTree::new();
        let create = create_collection_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Art",
            "ART",
            Some(10),
        );
        let id = coll_id(&p.handle(&create, &mut s).unwrap());

        let mint = Command {
            prism: "nft".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NftCommand::Mint {
                collection_id: id,
                to: PartyId::new("alice"),
                metadata_uri: "ipfs://x".into(),
                content_hash: Hash::digest(b"art"),
            })
            .unwrap(),
            nonce: 1,
        };
        p.handle(&mint, &mut s).unwrap();
        assert_eq!(owner_of(&s, &id, 0), Some(PartyId::new("alice")));

        let xfer = Command {
            prism: "nft".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NftCommand::Transfer {
                collection_id: id,
                token_index: 0,
                to: PartyId::new("bob"),
            })
            .unwrap(),
            nonce: 2,
        };
        p.handle(&xfer, &mut s).unwrap();
        assert_eq!(owner_of(&s, &id, 0), Some(PartyId::new("bob")));
    }

    #[test]
    fn non_owner_cannot_transfer() {
        let p = NftPrism::new();
        let mut s = StateTree::new();
        let create = create_collection_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Art",
            "ART",
            None,
        );
        let id = coll_id(&p.handle(&create, &mut s).unwrap());
        let mint = Command {
            prism: "nft".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NftCommand::Mint {
                collection_id: id,
                to: PartyId::new("alice"),
                metadata_uri: "ipfs://x".into(),
                content_hash: Hash::digest(b"a"),
            })
            .unwrap(),
            nonce: 1,
        };
        p.handle(&mint, &mut s).unwrap();
        let xfer = Command {
            prism: "nft".into(),
            submitter: PartyId::new("mallory"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NftCommand::Transfer {
                collection_id: id,
                token_index: 0,
                to: PartyId::new("mallory"),
            })
            .unwrap(),
            nonce: 0,
        };
        assert!(p.handle(&xfer, &mut s).is_err());
    }
}
