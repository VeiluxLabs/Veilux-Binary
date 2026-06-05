//! # Bridge Prism
//!
//! Guardian-attested cross-chain transfers between VEILUX and foreign chains
//! (Cosmos, Solana, Ethereum, or a custom chain). The trust model is a relayer
//! quorum, like Wormhole: a registered set of guardians watches both sides and
//! signs attestations; the bridge accepts an inbound transfer once a quorum of
//! valid Ed25519 signatures is present.
//!
//! ## Flows
//! - **Outbound** (`Send`): VEILUX tokens are locked (debited) here and an
//!   `OutboundLocked` event is emitted with a sequence number. Off-chain
//!   relayers observe it and mint/release on the foreign chain.
//! - **Inbound** (`Redeem`): relayers submit a foreign transfer plus guardian
//!   signatures. The bridge verifies quorum, checks the per-chain sequence
//!   (anti-replay), and credits the wrapped token to the recipient.
//!
//! ## Determinism & safety
//! Signature verification is deterministic, so every validator agrees. Replay
//! is prevented by a strictly-increasing per-chain sequence persisted in state.
//! Token balances reuse the Token Prism's `token/bal/<id>/<party>` keys
//! (decimal strings), so bridged value is real, spendable balance.

mod types;

pub use types::{BridgeConfig, ForeignChain, GuardianSignature, InboundTransfer, OutboundTransfer};

use serde::{Deserialize, Serialize};
use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

/// Token balance key — shared with the Token Prism so bridged tokens are
/// fungible with native ones.
fn bal_key(token_id: &Hash, who: &PartyId) -> String {
    format!("token/bal/{}/{}", token_id.to_hex(), who.0)
}

fn seq_key(chain: ForeignChain) -> String {
    format!("bridge/seq/{}", chain.as_u16())
}

fn out_seq_key(chain: ForeignChain) -> String {
    format!("bridge/outseq/{}", chain.as_u16())
}

fn get_balance(state: &StateTree, token_id: &Hash, who: &PartyId) -> u128 {
    state
        .get_json::<String>(&bal_key(token_id, who))
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn set_balance(
    state: &mut StateTree,
    token_id: &Hash,
    who: &PartyId,
    v: u128,
) -> Result<(), PrismError> {
    state
        .put_json(bal_key(token_id, who), &v.to_string())
        .map_err(|e| PrismError::Internal(e.to_string()))
}

/// Commands the Bridge Prism understands.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BridgeCommand {
    /// Register or update a foreign-chain bridge (admin only after creation).
    RegisterChain {
        chain: ForeignChain,
        guardians: Vec<String>,
        quorum: usize,
    },
    /// Lock VEILUX tokens to send them to a foreign chain.
    Send {
        chain: ForeignChain,
        recipient: String,
        token_id: Hash,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    /// Redeem a guardian-attested inbound transfer, minting wrapped tokens.
    Redeem {
        transfer: InboundTransfer,
        signatures: Vec<GuardianSignature>,
    },
}

mod u128_dec {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Events the Bridge Prism emits.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeEvent {
    ChainRegistered {
        chain: ForeignChain,
        guardians: usize,
        quorum: usize,
    },
    OutboundLocked {
        sequence: u64,
        chain: ForeignChain,
        recipient: String,
        token_id: Hash,
        #[serde(with = "u128_dec")]
        amount: u128,
        digest: Hash,
    },
    InboundRedeemed {
        chain: ForeignChain,
        sequence: u64,
        recipient: PartyId,
        token_id: Hash,
        #[serde(with = "u128_dec")]
        amount: u128,
        guardians_verified: usize,
    },
}

#[derive(Default)]
pub struct BridgePrism;

impl BridgePrism {
    pub fn new() -> Self {
        BridgePrism
    }

    fn load_config(state: &StateTree, chain: ForeignChain) -> Result<BridgeConfig, PrismError> {
        state
            .get_json::<BridgeConfig>(&BridgeConfig::config_key(chain))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("bridge for chain {}", chain.as_u16())))
    }

    fn event(cmd: &Command, payload: BridgeEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "bridge".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }

    /// Verify that at least `quorum` distinct registered guardians signed the
    /// digest. Each signature must be from a configured guardian, and no
    /// guardian is counted twice.
    fn verify_quorum(
        config: &BridgeConfig,
        digest: &Hash,
        signatures: &[GuardianSignature],
    ) -> Result<usize, PrismError> {
        let mut seen: Vec<String> = Vec::new();
        let mut valid = 0usize;
        for sig in signatures {
            if !config.guardians.contains(&sig.public_key) {
                continue; // not a registered guardian
            }
            if seen.contains(&sig.public_key) {
                continue; // duplicate guardian
            }
            let pk = hex_to_bytes(&sig.public_key)
                .ok_or_else(|| PrismError::InvalidPayload("bad guardian pubkey hex".into()))?;
            let sigbytes = hex_to_bytes(&sig.signature)
                .ok_or_else(|| PrismError::InvalidPayload("bad signature hex".into()))?;
            if veilux_veil::verify_bytes(&pk, digest.as_bytes(), &sigbytes).is_ok() {
                seen.push(sig.public_key.clone());
                valid += 1;
            }
        }
        if valid < config.quorum {
            return Err(PrismError::Unauthorized(format!(
                "guardian quorum not met: {valid}/{} valid (need {})",
                config.guardians.len(),
                config.quorum
            )));
        }
        Ok(valid)
    }
}

fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).ok()
}

impl Prism for BridgePrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "bridge",
            description: "Guardian-attested cross-chain transfers (Cosmos, Solana, EVM)",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: BridgeCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            BridgeCommand::RegisterChain {
                chain,
                guardians,
                quorum,
            } => {
                if quorum == 0 || quorum > guardians.len() {
                    return Err(PrismError::InvalidPayload(
                        "quorum must be in 1..=guardians.len()".into(),
                    ));
                }
                let key = BridgeConfig::config_key(chain);
                // If a config exists, only its admin may update it.
                if let Ok(Some(existing)) = state.get_json::<BridgeConfig>(&key) {
                    if existing.admin != command.submitter {
                        return Err(PrismError::Unauthorized(
                            "only bridge admin may update".into(),
                        ));
                    }
                }
                let config = BridgeConfig {
                    chain,
                    guardians: guardians.clone(),
                    quorum,
                    admin: command.submitter.clone(),
                };
                state
                    .put_json(key, &config)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        BridgeEvent::ChainRegistered {
                            chain,
                            guardians: guardians.len(),
                            quorum,
                        },
                    ),
                    5_000,
                ))
            }

            BridgeCommand::Send {
                chain,
                recipient,
                token_id,
                amount,
            } => {
                let _ = Self::load_config(state, chain)?;
                let sender = command.submitter.clone();
                let bal = get_balance(state, &token_id, &sender);
                if bal < amount {
                    return Err(PrismError::LimitExceeded(
                        "insufficient balance to bridge".into(),
                    ));
                }
                // Lock = debit sender (tokens are held by the bridge until the
                // foreign side mints; a real deployment escrows to a bridge
                // account — here we burn-on-send and mint-on-redeem).
                set_balance(state, &token_id, &sender, bal - amount)?;

                // Next outbound sequence.
                let seq: u64 = state
                    .get_json::<u64>(&out_seq_key(chain))
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                state
                    .put_json(out_seq_key(chain), &(seq + 1))
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let transfer = OutboundTransfer {
                    sequence: seq,
                    chain,
                    sender,
                    recipient: recipient.clone(),
                    token_id,
                    amount,
                };
                let digest = transfer.digest();

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        BridgeEvent::OutboundLocked {
                            sequence: seq,
                            chain,
                            recipient,
                            token_id,
                            amount,
                            digest,
                        },
                    ),
                    3_000,
                ))
            }

            BridgeCommand::Redeem {
                transfer,
                signatures,
            } => {
                let config = Self::load_config(state, transfer.chain)?;

                // Anti-replay: sequence must be exactly the next expected one.
                let expected: u64 = state
                    .get_json::<u64>(&seq_key(transfer.chain))
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                if transfer.sequence != expected {
                    return Err(PrismError::InvalidPayload(format!(
                        "bad inbound sequence: got {}, expected {expected}",
                        transfer.sequence
                    )));
                }

                let digest = transfer.digest();
                let verified = Self::verify_quorum(&config, &digest, &signatures)?;

                // Mint wrapped tokens to the recipient.
                let bal = get_balance(state, &transfer.token_id, &transfer.recipient);
                let new_bal = bal
                    .checked_add(transfer.amount)
                    .ok_or_else(|| PrismError::Internal("balance overflow".into()))?;
                set_balance(state, &transfer.token_id, &transfer.recipient, new_bal)?;

                // Advance the per-chain sequence.
                state
                    .put_json(seq_key(transfer.chain), &(expected + 1))
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        BridgeEvent::InboundRedeemed {
                            chain: transfer.chain,
                            sequence: transfer.sequence,
                            recipient: transfer.recipient,
                            token_id: transfer.token_id,
                            amount: transfer.amount,
                            guardians_verified: verified,
                        },
                    ),
                    8_000,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<BridgeCommand>(&command.payload) {
            Ok(BridgeCommand::RegisterChain { .. }) => 5_000,
            Ok(BridgeCommand::Send { .. }) => 3_000,
            Ok(BridgeCommand::Redeem { .. }) => 8_000,
            Err(_) => 1_000,
        }
    }
}

/// Build a `register_chain` command.
pub fn register_chain_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    chain: ForeignChain,
    guardians: Vec<String>,
    quorum: usize,
) -> Command {
    let payload = serde_json::to_vec(&BridgeCommand::RegisterChain {
        chain,
        guardians,
        quorum,
    })
    .unwrap_or_default();
    Command {
        prism: "bridge".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

/// Build a `send` (outbound lock) command.
pub fn send_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    chain: ForeignChain,
    recipient: &str,
    token_id: Hash,
    amount: u128,
) -> Command {
    let payload = serde_json::to_vec(&BridgeCommand::Send {
        chain,
        recipient: recipient.to_string(),
        token_id,
        amount,
    })
    .unwrap_or_default();
    Command {
        prism: "bridge".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_veil::PartyIdentity;

    fn seed_id(seed: u8) -> PartyIdentity {
        PartyIdentity::from_seed("guardian", &[seed; 32])
    }

    fn register(
        state: &mut StateTree,
        prism: &BridgePrism,
        chain: ForeignChain,
        guardian_pubs: Vec<String>,
        quorum: usize,
    ) {
        let cmd = register_chain_command(
            PartyId::new("admin"),
            Visibility::Public,
            0,
            chain,
            guardian_pubs,
            quorum,
        );
        prism.handle(&cmd, state).unwrap();
    }

    #[test]
    fn register_and_redeem_with_quorum() {
        let prism = BridgePrism::new();
        let mut state = StateTree::new();

        let g1 = seed_id(11);
        let g2 = seed_id(22);
        let g3 = seed_id(33);
        register(
            &mut state,
            &prism,
            ForeignChain::Cosmos,
            vec![
                hex::encode(g1.public_key()),
                hex::encode(g2.public_key()),
                hex::encode(g3.public_key()),
            ],
            2,
        );

        let token_id = Hash::digest(b"wrapped-atom");
        let transfer = InboundTransfer {
            chain: ForeignChain::Cosmos,
            sequence: 0,
            foreign_sender: "cosmos1abc".into(),
            recipient: PartyId::new("alice"),
            token_id,
            amount: 1_000,
        };
        let digest = transfer.digest();

        // Two of three guardians sign -> meets quorum.
        let signatures = vec![
            GuardianSignature {
                public_key: hex::encode(g1.public_key()),
                signature: hex::encode(g1.sign_bytes(digest.as_bytes())),
            },
            GuardianSignature {
                public_key: hex::encode(g2.public_key()),
                signature: hex::encode(g2.sign_bytes(digest.as_bytes())),
            },
        ];

        let redeem = Command {
            prism: "bridge".into(),
            submitter: PartyId::new("relayer"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&BridgeCommand::Redeem {
                transfer,
                signatures,
            })
            .unwrap(),
            nonce: 0,
        };
        prism.handle(&redeem, &mut state).unwrap();

        assert_eq!(
            get_balance(&state, &token_id, &PartyId::new("alice")),
            1_000
        );
    }

    #[test]
    fn redeem_fails_below_quorum() {
        let prism = BridgePrism::new();
        let mut state = StateTree::new();
        let g1 = seed_id(11);
        let g2 = seed_id(22);
        register(
            &mut state,
            &prism,
            ForeignChain::Cosmos,
            vec![hex::encode(g1.public_key()), hex::encode(g2.public_key())],
            2,
        );

        let token_id = Hash::digest(b"wrapped");
        let transfer = InboundTransfer {
            chain: ForeignChain::Cosmos,
            sequence: 0,
            foreign_sender: "cosmos1x".into(),
            recipient: PartyId::new("alice"),
            token_id,
            amount: 500,
        };
        let digest = transfer.digest();
        // Only one signature -> below quorum of 2.
        let signatures = vec![GuardianSignature {
            public_key: hex::encode(g1.public_key()),
            signature: hex::encode(g1.sign_bytes(digest.as_bytes())),
        }];
        let redeem = Command {
            prism: "bridge".into(),
            submitter: PartyId::new("relayer"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&BridgeCommand::Redeem {
                transfer,
                signatures,
            })
            .unwrap(),
            nonce: 0,
        };
        assert!(prism.handle(&redeem, &mut state).is_err());
    }

    #[test]
    fn replay_is_rejected() {
        let prism = BridgePrism::new();
        let mut state = StateTree::new();
        let g1 = seed_id(11);
        let g2 = seed_id(22);
        register(
            &mut state,
            &prism,
            ForeignChain::Cosmos,
            vec![hex::encode(g1.public_key()), hex::encode(g2.public_key())],
            2,
        );

        let token_id = Hash::digest(b"w");
        let make = |seq: u64| {
            let transfer = InboundTransfer {
                chain: ForeignChain::Cosmos,
                sequence: seq,
                foreign_sender: "c1".into(),
                recipient: PartyId::new("alice"),
                token_id,
                amount: 100,
            };
            let digest = transfer.digest();
            let signatures = vec![
                GuardianSignature {
                    public_key: hex::encode(g1.public_key()),
                    signature: hex::encode(g1.sign_bytes(digest.as_bytes())),
                },
                GuardianSignature {
                    public_key: hex::encode(g2.public_key()),
                    signature: hex::encode(g2.sign_bytes(digest.as_bytes())),
                },
            ];
            Command {
                prism: "bridge".into(),
                submitter: PartyId::new("relayer"),
                visibility: Visibility::Public,
                payload: serde_json::to_vec(&BridgeCommand::Redeem {
                    transfer,
                    signatures,
                })
                .unwrap(),
                nonce: 0,
            }
        };

        prism.handle(&make(0), &mut state).unwrap();
        // Replaying sequence 0 must fail (expected is now 1).
        assert!(prism.handle(&make(0), &mut state).is_err());
        // Correct next sequence works.
        prism.handle(&make(1), &mut state).unwrap();
        assert_eq!(get_balance(&state, &token_id, &PartyId::new("alice")), 200);
    }

    #[test]
    fn send_locks_balance() {
        let prism = BridgePrism::new();
        let mut state = StateTree::new();
        let g1 = seed_id(11);

        let token_id = Hash::digest(b"native");
        set_balance(&mut state, &token_id, &PartyId::new("alice"), 1_000).unwrap();

        let send = send_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            ForeignChain::Solana,
            "SoLaNaAddr111",
            token_id,
            400,
        );
        // Solana bridge not registered -> should fail.
        assert!(prism.handle(&send, &mut state).is_err());

        // Register Solana, then send works.
        register(
            &mut state,
            &prism,
            ForeignChain::Solana,
            vec![hex::encode(g1.public_key())],
            1,
        );
        prism.handle(&send, &mut state).unwrap();
        assert_eq!(get_balance(&state, &token_id, &PartyId::new("alice")), 600);
    }
}
