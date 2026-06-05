use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};
use veilux_veil::verify_bytes;

const FEED_PREFIX: &str = "oracle/feed/";
const VALUE_PREFIX: &str = "oracle/value/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Feed {
    pub feed_id: Hash,
    pub name: String,
    pub admin: PartyId,
    pub reporters: Vec<String>,
    pub quorum: u32,
    pub round: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedValue {
    pub feed_id: Hash,
    pub round: u64,
    pub value: Vec<u8>,
    pub reporters: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReporterSig {
    pub public_key: String,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OracleCommand {
    RegisterFeed {
        name: String,
        reporters: Vec<String>,
        quorum: u32,
    },
    Report {
        feed_id: Hash,
        round: u64,
        value: Vec<u8>,
        signatures: Vec<ReporterSig>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OracleEvent {
    FeedRegistered {
        feed_id: Hash,
        name: String,
        quorum: u32,
    },
    ValueUpdated {
        feed_id: Hash,
        round: u64,
        value: Vec<u8>,
        signers: u32,
    },
}

pub fn report_digest(feed_id: &Hash, round: u64, value: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(64 + value.len());
    m.extend_from_slice(b"veilux/oracle/report/v1");
    m.push(0xff);
    m.extend_from_slice(feed_id.as_bytes());
    m.extend_from_slice(&round.to_le_bytes());
    m.push(0xff);
    m.extend_from_slice(value);
    m
}

#[derive(Default)]
pub struct OraclePrism;

impl OraclePrism {
    pub fn new() -> Self {
        OraclePrism
    }

    fn feed_key(id: &Hash) -> String {
        format!("{FEED_PREFIX}{}", id.to_hex())
    }

    fn value_key(id: &Hash) -> String {
        format!("{VALUE_PREFIX}{}", id.to_hex())
    }

    fn load_feed(state: &StateTree, id: &Hash) -> Result<Feed, PrismError> {
        state
            .get_json::<Feed>(&Self::feed_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("feed {}", id.to_hex())))
    }

    fn event(cmd: &Command, payload: OracleEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "oracle".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for OraclePrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "oracle",
            description:
                "Quorum-attested external data feeds (prices, AI outputs, off-chain facts)",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: OracleCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            OracleCommand::RegisterFeed {
                name,
                reporters,
                quorum,
            } => {
                if quorum == 0 || (quorum as usize) > reporters.len() {
                    return Err(PrismError::InvalidPayload(
                        "quorum must be in 1..=reporters".into(),
                    ));
                }
                let feed_id = Hash::commit(
                    "oracle/feed",
                    &[command.submitter.0.as_bytes(), name.as_bytes()],
                );
                if state.contains(&Self::feed_key(&feed_id)) {
                    return Err(PrismError::InvalidPayload("feed already exists".into()));
                }
                let feed = Feed {
                    feed_id,
                    name: name.clone(),
                    admin: command.submitter.clone(),
                    reporters,
                    quorum,
                    round: 0,
                };
                state
                    .put_json(Self::feed_key(&feed_id), &feed)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        OracleEvent::FeedRegistered {
                            feed_id,
                            name,
                            quorum,
                        },
                    ),
                    5_000,
                ))
            }

            OracleCommand::Report {
                feed_id,
                round,
                value,
                signatures,
            } => {
                let mut feed = Self::load_feed(state, &feed_id)?;
                if round <= feed.round {
                    return Err(PrismError::InvalidPayload(
                        "round must strictly advance".into(),
                    ));
                }
                let digest = report_digest(&feed_id, round, &value);
                let mut counted: Vec<String> = Vec::new();
                for sig in &signatures {
                    if !feed.reporters.contains(&sig.public_key) {
                        continue;
                    }
                    if counted.contains(&sig.public_key) {
                        continue;
                    }
                    let pk = match hex::decode(sig.public_key.trim_start_matches("0x")) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };
                    let sg = match hex::decode(sig.signature.trim_start_matches("0x")) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };
                    if verify_bytes(&pk, &digest, &sg).is_ok() {
                        counted.push(sig.public_key.clone());
                    }
                }
                if (counted.len() as u32) < feed.quorum {
                    return Err(PrismError::Unauthorized(format!(
                        "insufficient reporter quorum: {}/{}",
                        counted.len(),
                        feed.quorum
                    )));
                }
                feed.round = round;
                state
                    .put_json(Self::feed_key(&feed_id), &feed)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                let fv = FeedValue {
                    feed_id,
                    round,
                    value: value.clone(),
                    reporters: counted.clone(),
                };
                state
                    .put_json(Self::value_key(&feed_id), &fv)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        OracleEvent::ValueUpdated {
                            feed_id,
                            round,
                            value,
                            signers: counted.len() as u32,
                        },
                    ),
                    6_000,
                ))
            }
        }
    }

    fn estimate(&self, _command: &Command, _state: &StateTree) -> u64 {
        6_000
    }
}

pub fn latest_value(state: &StateTree, feed_id: &Hash) -> Option<FeedValue> {
    state
        .get_json::<FeedValue>(&OraclePrism::value_key(feed_id))
        .ok()
        .flatten()
}

pub fn register_feed_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    reporters: Vec<String>,
    quorum: u32,
) -> Command {
    let payload = serde_json::to_vec(&OracleCommand::RegisterFeed {
        name: name.to_string(),
        reporters,
        quorum,
    })
    .unwrap_or_default();
    Command {
        prism: "oracle".into(),
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

    fn reporter(seed: u8) -> PartyIdentity {
        PartyIdentity::from_seed("reporter", &[seed; 32])
    }

    fn feed_id_from(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<OracleEvent>(&out.events[0].payload).unwrap() {
            OracleEvent::FeedRegistered { feed_id, .. } => feed_id,
            _ => panic!("expected FeedRegistered"),
        }
    }

    fn register(p: &OraclePrism, s: &mut StateTree, keys: &[&PartyIdentity], quorum: u32) -> Hash {
        let reporters: Vec<String> = keys.iter().map(|k| hex::encode(k.public_key())).collect();
        let cmd = register_feed_command(
            PartyId::new("admin"),
            Visibility::Public,
            0,
            "BTC/USD",
            reporters,
            quorum,
        );
        feed_id_from(&p.handle(&cmd, s).unwrap())
    }

    fn report_cmd(
        feed_id: Hash,
        round: u64,
        value: Vec<u8>,
        signers: &[&PartyIdentity],
    ) -> Command {
        let digest = report_digest(&feed_id, round, &value);
        let signatures = signers
            .iter()
            .map(|k| ReporterSig {
                public_key: hex::encode(k.public_key()),
                signature: hex::encode(k.sign_bytes(&digest)),
            })
            .collect();
        let payload = serde_json::to_vec(&OracleCommand::Report {
            feed_id,
            round,
            value,
            signatures,
        })
        .unwrap();
        Command {
            prism: "oracle".into(),
            submitter: PartyId::new("relayer"),
            visibility: Visibility::Public,
            payload,
            nonce: round,
        }
    }

    #[test]
    fn quorum_report_is_accepted() {
        let p = OraclePrism::new();
        let mut s = StateTree::new();
        let r1 = reporter(1);
        let r2 = reporter(2);
        let r3 = reporter(3);
        let feed = register(&p, &mut s, &[&r1, &r2, &r3], 2);

        let cmd = report_cmd(feed, 1, 42_000u64.to_le_bytes().to_vec(), &[&r1, &r2]);
        p.handle(&cmd, &mut s).unwrap();
        let v = latest_value(&s, &feed).unwrap();
        assert_eq!(v.round, 1);
        assert_eq!(v.value, 42_000u64.to_le_bytes().to_vec());
    }

    #[test]
    fn below_quorum_is_rejected() {
        let p = OraclePrism::new();
        let mut s = StateTree::new();
        let r1 = reporter(1);
        let r2 = reporter(2);
        let r3 = reporter(3);
        let feed = register(&p, &mut s, &[&r1, &r2, &r3], 2);

        let cmd = report_cmd(feed, 1, vec![1, 2, 3], &[&r1]);
        assert!(p.handle(&cmd, &mut s).is_err());
    }

    #[test]
    fn stale_round_is_rejected() {
        let p = OraclePrism::new();
        let mut s = StateTree::new();
        let r1 = reporter(1);
        let r2 = reporter(2);
        let feed = register(&p, &mut s, &[&r1, &r2], 2);

        p.handle(&report_cmd(feed, 5, vec![9], &[&r1, &r2]), &mut s)
            .unwrap();
        assert!(p
            .handle(&report_cmd(feed, 5, vec![9], &[&r1, &r2]), &mut s)
            .is_err());
        assert!(p
            .handle(&report_cmd(feed, 3, vec![9], &[&r1, &r2]), &mut s)
            .is_err());
    }

    #[test]
    fn forged_signature_does_not_count() {
        let p = OraclePrism::new();
        let mut s = StateTree::new();
        let r1 = reporter(1);
        let r2 = reporter(2);
        let outsider = reporter(9);
        let feed = register(&p, &mut s, &[&r1, &r2], 2);

        let cmd = report_cmd(feed, 1, vec![7], &[&r1, &outsider]);
        assert!(p.handle(&cmd, &mut s).is_err());
    }
}
