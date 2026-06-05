use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

mod u128_dec {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<u128>().map_err(serde::de::Error::custom)
    }
}

const TOKEN_PREFIX: &str = "confidential/token/";
const NOTE_PREFIX: &str = "confidential/note/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfTokenMeta {
    pub token_id: Hash,
    pub name: String,
    pub symbol: String,
    pub admin: PartyId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteState {
    pub token_id: Hash,
    pub spent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteOpening {
    pub owner: PartyId,
    #[serde(with = "u128_dec")]
    pub amount: u128,
    pub blinding: String,
}

pub fn note_commitment(token_id: &Hash, opening: &NoteOpening) -> Hash {
    Hash::commit(
        "veilux/conf/note",
        &[
            token_id.as_bytes(),
            opening.owner.0.as_bytes(),
            &opening.amount.to_le_bytes(),
            opening.blinding.as_bytes(),
        ],
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ConfidentialCommand {
    CreateToken {
        name: String,
        symbol: String,
    },
    Mint {
        token_id: Hash,
        opening: NoteOpening,
    },
    Transfer {
        token_id: Hash,
        input: NoteOpening,
        outputs: Vec<NoteOpening>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfidentialEvent {
    TokenCreated {
        token_id: Hash,
        symbol: String,
    },
    NoteCreated {
        token_id: Hash,
        commitment: Hash,
    },
    NoteSpent {
        token_id: Hash,
        commitment: Hash,
        outputs: Vec<Hash>,
    },
}

#[derive(Default)]
pub struct ConfidentialPrism;

impl ConfidentialPrism {
    pub fn new() -> Self {
        ConfidentialPrism
    }

    fn token_key(id: &Hash) -> String {
        format!("{TOKEN_PREFIX}{}", id.to_hex())
    }

    fn note_key(commitment: &Hash) -> String {
        format!("{NOTE_PREFIX}{}", commitment.to_hex())
    }

    fn load_token(state: &StateTree, id: &Hash) -> Result<ConfTokenMeta, PrismError> {
        state
            .get_json::<ConfTokenMeta>(&Self::token_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("conf token {}", id.to_hex())))
    }

    fn note(state: &StateTree, commitment: &Hash) -> Option<NoteState> {
        state
            .get_json::<NoteState>(&Self::note_key(commitment))
            .ok()
            .flatten()
    }

    fn put_note(
        state: &mut StateTree,
        commitment: &Hash,
        note: &NoteState,
    ) -> Result<(), PrismError> {
        state
            .put_json(Self::note_key(commitment), note)
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn event(cmd: &Command, payload: ConfidentialEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "confidential".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for ConfidentialPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "confidential",
            description:
                "Confidential token: hidden balances via note commitments + selective disclosure",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: ConfidentialCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            ConfidentialCommand::CreateToken { name, symbol } => {
                let token_id = Hash::commit(
                    "confidential/token-id",
                    &[command.submitter.0.as_bytes(), symbol.as_bytes()],
                );
                if state.contains(&Self::token_key(&token_id)) {
                    return Err(PrismError::InvalidPayload("token already exists".into()));
                }
                let meta = ConfTokenMeta {
                    token_id,
                    name,
                    symbol: symbol.clone(),
                    admin: command.submitter.clone(),
                };
                state
                    .put_json(Self::token_key(&token_id), &meta)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        ConfidentialEvent::TokenCreated { token_id, symbol },
                    ),
                    5_000,
                ))
            }

            ConfidentialCommand::Mint { token_id, opening } => {
                let meta = Self::load_token(state, &token_id)?;
                if meta.admin != command.submitter {
                    return Err(PrismError::Unauthorized("only admin can mint".into()));
                }
                if opening.amount == 0 {
                    return Err(PrismError::InvalidPayload("amount must be > 0".into()));
                }
                let commitment = note_commitment(&token_id, &opening);
                if Self::note(state, &commitment).is_some() {
                    return Err(PrismError::InvalidPayload("note already exists".into()));
                }
                Self::put_note(
                    state,
                    &commitment,
                    &NoteState {
                        token_id,
                        spent: false,
                    },
                )?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        ConfidentialEvent::NoteCreated {
                            token_id,
                            commitment,
                        },
                    ),
                    4_000,
                ))
            }

            ConfidentialCommand::Transfer {
                token_id,
                input,
                outputs,
            } => {
                let _ = Self::load_token(state, &token_id)?;
                if outputs.is_empty() {
                    return Err(PrismError::InvalidPayload("at least one output".into()));
                }
                if input.owner != command.submitter {
                    return Err(PrismError::Unauthorized(
                        "only the note owner can spend it".into(),
                    ));
                }
                let in_commit = note_commitment(&token_id, &input);
                let note = Self::note(state, &in_commit)
                    .ok_or_else(|| PrismError::NotFound("input note".into()))?;
                if note.spent {
                    return Err(PrismError::InvalidPayload("note already spent".into()));
                }
                let out_sum: u128 = outputs
                    .iter()
                    .try_fold(0u128, |acc, o| acc.checked_add(o.amount))
                    .ok_or_else(|| PrismError::Internal("output sum overflow".into()))?;
                if out_sum != input.amount {
                    return Err(PrismError::InvalidPayload(
                        "outputs do not conserve value".into(),
                    ));
                }
                let mut out_commits = Vec::with_capacity(outputs.len());
                for o in &outputs {
                    if o.amount == 0 {
                        return Err(PrismError::InvalidPayload("zero-value output".into()));
                    }
                    let c = note_commitment(&token_id, o);
                    if Self::note(state, &c).is_some() || out_commits.contains(&c) {
                        return Err(PrismError::InvalidPayload("output note collision".into()));
                    }
                    out_commits.push(c);
                }
                Self::put_note(
                    state,
                    &in_commit,
                    &NoteState {
                        token_id,
                        spent: true,
                    },
                )?;
                for c in &out_commits {
                    Self::put_note(
                        state,
                        c,
                        &NoteState {
                            token_id,
                            spent: false,
                        },
                    )?;
                }
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        ConfidentialEvent::NoteSpent {
                            token_id,
                            commitment: in_commit,
                            outputs: out_commits,
                        },
                    ),
                    8_000,
                ))
            }
        }
    }

    fn estimate(&self, _command: &Command, _state: &StateTree) -> u64 {
        6_000
    }
}

pub fn disclose_note(state: &StateTree, token_id: &Hash, opening: &NoteOpening) -> Option<bool> {
    let commitment = note_commitment(token_id, opening);
    ConfidentialPrism::note(state, &commitment).map(|n| n.spent)
}

pub fn create_token_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    symbol: &str,
) -> Command {
    let payload = serde_json::to_vec(&ConfidentialCommand::CreateToken {
        name: name.to_string(),
        symbol: symbol.to_string(),
    })
    .unwrap_or_default();
    Command {
        prism: "confidential".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opening(owner: &str, amount: u128, blind: &str) -> NoteOpening {
        NoteOpening {
            owner: PartyId::new(owner),
            amount,
            blinding: blind.to_string(),
        }
    }

    fn create(p: &ConfidentialPrism, s: &mut StateTree) -> Hash {
        let cmd = create_token_command(
            PartyId::new("admin"),
            Visibility::Public,
            0,
            "Private Dollar",
            "pUSD",
        );
        match serde_json::from_slice::<ConfidentialEvent>(
            &p.handle(&cmd, s).unwrap().events[0].payload,
        )
        .unwrap()
        {
            ConfidentialEvent::TokenCreated { token_id, .. } => token_id,
            _ => panic!("expected TokenCreated"),
        }
    }

    fn mint(p: &ConfidentialPrism, s: &mut StateTree, token: Hash, o: &NoteOpening, nonce: u64) {
        let payload = serde_json::to_vec(&ConfidentialCommand::Mint {
            token_id: token,
            opening: o.clone(),
        })
        .unwrap();
        let cmd = Command {
            prism: "confidential".into(),
            submitter: PartyId::new("admin"),
            visibility: Visibility::Parties(vec![PartyId::new("admin"), o.owner.clone()]),
            payload,
            nonce,
        };
        p.handle(&cmd, s).unwrap();
    }

    #[test]
    fn balances_never_appear_in_public_state() {
        let p = ConfidentialPrism::new();
        let mut s = StateTree::new();
        let token = create(&p, &mut s);
        mint(&p, &mut s, token, &opening("alice", 1_000, "r1"), 1);

        let notes: Vec<_> = s.iter_prefix("confidential/note/").collect();
        assert_eq!(notes.len(), 1);
        let commitment = note_commitment(&token, &opening("alice", 1_000, "r1"));
        assert!(disclose_note(&s, &token, &opening("alice", 1_000, "r1")).is_some());
        assert!(disclose_note(&s, &token, &opening("alice", 999, "r1")).is_none());
        let _ = commitment;
    }

    #[test]
    fn confidential_transfer_conserves_value() {
        let p = ConfidentialPrism::new();
        let mut s = StateTree::new();
        let token = create(&p, &mut s);
        let note = opening("alice", 1_000, "r1");
        mint(&p, &mut s, token, &note, 1);

        let out_bob = opening("bob", 700, "r2");
        let out_change = opening("alice", 300, "r3");
        let payload = serde_json::to_vec(&ConfidentialCommand::Transfer {
            token_id: token,
            input: note.clone(),
            outputs: vec![out_bob.clone(), out_change.clone()],
        })
        .unwrap();
        let cmd = Command {
            prism: "confidential".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Parties(vec![PartyId::new("alice"), PartyId::new("bob")]),
            payload,
            nonce: 2,
        };
        p.handle(&cmd, &mut s).unwrap();

        assert_eq!(disclose_note(&s, &token, &note), Some(true));
        assert_eq!(disclose_note(&s, &token, &out_bob), Some(false));
        assert_eq!(disclose_note(&s, &token, &out_change), Some(false));
    }

    #[test]
    fn non_conserving_transfer_is_rejected() {
        let p = ConfidentialPrism::new();
        let mut s = StateTree::new();
        let token = create(&p, &mut s);
        let note = opening("alice", 1_000, "r1");
        mint(&p, &mut s, token, &note, 1);

        let payload = serde_json::to_vec(&ConfidentialCommand::Transfer {
            token_id: token,
            input: note.clone(),
            outputs: vec![opening("bob", 1_100, "r2")],
        })
        .unwrap();
        let cmd = Command {
            prism: "confidential".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload,
            nonce: 2,
        };
        assert!(p.handle(&cmd, &mut s).is_err());
    }

    #[test]
    fn double_spend_is_rejected() {
        let p = ConfidentialPrism::new();
        let mut s = StateTree::new();
        let token = create(&p, &mut s);
        let note = opening("alice", 500, "r1");
        mint(&p, &mut s, token, &note, 1);

        let spend = |nonce: u64| {
            let payload = serde_json::to_vec(&ConfidentialCommand::Transfer {
                token_id: token,
                input: note.clone(),
                outputs: vec![opening("bob", 500, &format!("out{nonce}"))],
            })
            .unwrap();
            Command {
                prism: "confidential".into(),
                submitter: PartyId::new("alice"),
                visibility: Visibility::Public,
                payload,
                nonce,
            }
        };
        p.handle(&spend(2), &mut s).unwrap();
        assert!(p.handle(&spend(3), &mut s).is_err());
    }

    #[test]
    fn cannot_spend_someone_elses_note() {
        let p = ConfidentialPrism::new();
        let mut s = StateTree::new();
        let token = create(&p, &mut s);
        let note = opening("alice", 500, "r1");
        mint(&p, &mut s, token, &note, 1);

        let payload = serde_json::to_vec(&ConfidentialCommand::Transfer {
            token_id: token,
            input: note.clone(),
            outputs: vec![opening("bob", 500, "r2")],
        })
        .unwrap();
        let cmd = Command {
            prism: "confidential".into(),
            submitter: PartyId::new("mallory"),
            visibility: Visibility::Public,
            payload,
            nonce: 0,
        };
        assert!(p.handle(&cmd, &mut s).is_err());
    }
}
