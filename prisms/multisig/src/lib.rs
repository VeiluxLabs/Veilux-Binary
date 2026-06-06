use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

const ACC_PREFIX: &str = "multisig/acc/";
const TX_PREFIX: &str = "multisig/tx/";
const MAX_OWNERS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MultisigAccount {
    pub id: Hash,
    pub owners: Vec<PartyId>,
    pub threshold: u32,
    pub nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingTx {
    pub account: Hash,
    pub seq: u64,
    pub inner: Command,
    pub confirmations: BTreeSet<String>,
    pub executed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MultisigCommand {
    CreateAccount {
        owners: Vec<PartyId>,
        threshold: u32,
    },
    Propose {
        account: Hash,
        inner: Box<Command>,
    },
    Confirm {
        account: Hash,
        seq: u64,
    },
    Revoke {
        account: Hash,
        seq: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MultisigEvent {
    AccountCreated {
        account: Hash,
        threshold: u32,
        owners: usize,
    },
    Proposed {
        account: Hash,
        seq: u64,
        proposer: PartyId,
    },
    Confirmed {
        account: Hash,
        seq: u64,
        by: PartyId,
        confirmations: u32,
    },
    Revoked {
        account: Hash,
        seq: u64,
        by: PartyId,
    },
    Executed {
        account: Hash,
        seq: u64,
    },
}

#[derive(Default)]
pub struct MultisigPrism;

impl MultisigPrism {
    pub fn new() -> Self {
        MultisigPrism
    }

    fn acc_key(id: &Hash) -> String {
        format!("{ACC_PREFIX}{}", id.to_hex())
    }

    fn tx_key(account: &Hash, seq: u64) -> String {
        format!("{TX_PREFIX}{}/{}", account.to_hex(), seq)
    }

    fn load_account(state: &StateTree, id: &Hash) -> Result<MultisigAccount, PrismError> {
        state
            .get_json::<MultisigAccount>(&Self::acc_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("multisig account {}", id.to_hex())))
    }

    fn load_tx(state: &StateTree, account: &Hash, seq: u64) -> Result<PendingTx, PrismError> {
        state
            .get_json::<PendingTx>(&Self::tx_key(account, seq))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("multisig tx {seq}")))
    }

    fn event(cmd: &Command, payload: MultisigEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "multisig".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for MultisigPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "multisig",
            description: "M-of-N shared accounts: propose, confirm and execute commands",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: MultisigCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;
        let caller = command.submitter.clone();

        match cmd {
            MultisigCommand::CreateAccount { owners, threshold } => {
                let mut unique: Vec<PartyId> = Vec::new();
                for o in owners {
                    if !unique.contains(&o) {
                        unique.push(o);
                    }
                }
                if unique.is_empty() || unique.len() > MAX_OWNERS {
                    return Err(PrismError::InvalidPayload(
                        "owners must be 1..=64 distinct parties".into(),
                    ));
                }
                if threshold == 0 || threshold as usize > unique.len() {
                    return Err(PrismError::InvalidPayload(
                        "threshold must be 1..=owners".into(),
                    ));
                }
                let id = Hash::commit(
                    "multisig/acc-id",
                    &[
                        caller.0.as_bytes(),
                        &threshold.to_be_bytes(),
                        unique
                            .iter()
                            .map(|p| p.0.clone())
                            .collect::<Vec<_>>()
                            .join(",")
                            .as_bytes(),
                    ],
                );
                if state.contains(&Self::acc_key(&id)) {
                    return Err(PrismError::InvalidPayload("account already exists".into()));
                }
                let owners_len = unique.len();
                let account = MultisigAccount {
                    id,
                    owners: unique,
                    threshold,
                    nonce: 0,
                };
                state
                    .put_json(Self::acc_key(&id), &account)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        MultisigEvent::AccountCreated {
                            account: id,
                            threshold,
                            owners: owners_len,
                        },
                    ),
                    4_000,
                ))
            }

            MultisigCommand::Propose { account, inner } => {
                let mut acc = Self::load_account(state, &account)?;
                if !acc.owners.contains(&caller) {
                    return Err(PrismError::Unauthorized("not an account owner".into()));
                }
                let seq = acc.nonce;
                acc.nonce += 1;
                let mut confirmations = BTreeSet::new();
                confirmations.insert(caller.0.clone());
                let reached = acc.threshold <= 1;
                let pending = PendingTx {
                    account,
                    seq,
                    inner: (*inner).clone(),
                    confirmations,
                    executed: reached,
                };
                state
                    .put_json(Self::acc_key(&account), &acc)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                state
                    .put_json(Self::tx_key(&account, seq), &pending)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let mut events = vec![Self::event(
                    command,
                    MultisigEvent::Proposed {
                        account,
                        seq,
                        proposer: caller,
                    },
                )];
                let mut derived = Vec::new();
                if reached {
                    events.push(Self::event(command, MultisigEvent::Executed { account, seq }));
                    derived.push((*inner).clone());
                }
                Ok(PrismOutput {
                    events,
                    derived_commands: derived,
                    cost: 3_000,
                })
            }

            MultisigCommand::Confirm { account, seq } => {
                let acc = Self::load_account(state, &account)?;
                if !acc.owners.contains(&caller) {
                    return Err(PrismError::Unauthorized("not an account owner".into()));
                }
                let mut tx = Self::load_tx(state, &account, seq)?;
                if tx.executed {
                    return Err(PrismError::InvalidPayload("tx already executed".into()));
                }
                tx.confirmations.insert(caller.0.clone());
                let count = tx.confirmations.len() as u32;
                let reached = count >= acc.threshold;
                let mut events = vec![Self::event(
                    command,
                    MultisigEvent::Confirmed {
                        account,
                        seq,
                        by: caller,
                        confirmations: count,
                    },
                )];
                let mut derived = Vec::new();
                if reached {
                    tx.executed = true;
                    events.push(Self::event(command, MultisigEvent::Executed { account, seq }));
                    derived.push(tx.inner.clone());
                }
                state
                    .put_json(Self::tx_key(&account, seq), &tx)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput {
                    events,
                    derived_commands: derived,
                    cost: 1_500,
                })
            }

            MultisigCommand::Revoke { account, seq } => {
                let acc = Self::load_account(state, &account)?;
                if !acc.owners.contains(&caller) {
                    return Err(PrismError::Unauthorized("not an account owner".into()));
                }
                let mut tx = Self::load_tx(state, &account, seq)?;
                if tx.executed {
                    return Err(PrismError::InvalidPayload(
                        "cannot revoke an executed tx".into(),
                    ));
                }
                tx.confirmations.remove(&caller.0);
                state
                    .put_json(Self::tx_key(&account, seq), &tx)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(command, MultisigEvent::Revoked { account, seq, by: caller }),
                    800,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<MultisigCommand>(&command.payload) {
            Ok(MultisigCommand::CreateAccount { .. }) => 4_000,
            Ok(MultisigCommand::Propose { .. }) => 3_000,
            Ok(MultisigCommand::Confirm { .. }) => 1_500,
            Ok(_) => 800,
            Err(_) => 1_000,
        }
    }
}

pub fn account_id(state: &StateTree, id: &Hash) -> Option<MultisigAccount> {
    state.get_json(&MultisigPrism::acc_key(id)).ok().flatten()
}

pub fn pending_tx(state: &StateTree, account: &Hash, seq: u64) -> Option<PendingTx> {
    state
        .get_json(&MultisigPrism::tx_key(account, seq))
        .ok()
        .flatten()
}

pub fn create_account_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    owners: Vec<PartyId>,
    threshold: u32,
) -> Command {
    let payload = serde_json::to_vec(&MultisigCommand::CreateAccount { owners, threshold })
        .unwrap_or_default();
    Command {
        prism: "multisig".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn propose_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    account: Hash,
    inner: Command,
) -> Command {
    let payload = serde_json::to_vec(&MultisigCommand::Propose {
        account,
        inner: Box::new(inner),
    })
    .unwrap_or_default();
    Command {
        prism: "multisig".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn confirm_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    account: Hash,
    seq: u64,
) -> Command {
    let payload =
        serde_json::to_vec(&MultisigCommand::Confirm { account, seq }).unwrap_or_default();
    Command {
        prism: "multisig".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(owners: &[&str], threshold: u32) -> Command {
        create_account_command(
            PartyId::new(owners[0]),
            Visibility::Public,
            0,
            owners.iter().map(|o| PartyId::new(*o)).collect(),
            threshold,
        )
    }

    fn dummy_inner() -> Command {
        Command {
            prism: "token".into(),
            submitter: PartyId::new("multisig"),
            visibility: Visibility::Public,
            payload: b"{}".to_vec(),
            nonce: 0,
        }
    }

    fn acc_id(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<MultisigEvent>(&out.events[0].payload).unwrap() {
            MultisigEvent::AccountCreated { account, .. } => account,
            _ => panic!("expected AccountCreated"),
        }
    }

    #[test]
    fn two_of_three_executes_on_second_confirm() {
        let p = MultisigPrism::new();
        let mut s = StateTree::new();
        let id = acc_id(&p.handle(&create(&["a", "b", "c"], 2), &mut s).unwrap());

        let propose = propose_command(
            PartyId::new("a"),
            Visibility::Public,
            1,
            id,
            dummy_inner(),
        );
        let out = p.handle(&propose, &mut s).unwrap();
        assert!(
            out.derived_commands.is_empty(),
            "1 confirmation must not reach a 2-of-3 threshold"
        );

        let confirm = confirm_command(PartyId::new("b"), Visibility::Public, 0, id, 0);
        let out2 = p.handle(&confirm, &mut s).unwrap();
        assert_eq!(
            out2.derived_commands.len(),
            1,
            "second confirmation must dispatch the inner command"
        );
        assert!(pending_tx(&s, &id, 0).unwrap().executed);
    }

    #[test]
    fn one_of_n_executes_immediately() {
        let p = MultisigPrism::new();
        let mut s = StateTree::new();
        let id = acc_id(&p.handle(&create(&["a", "b"], 1), &mut s).unwrap());
        let propose =
            propose_command(PartyId::new("a"), Visibility::Public, 1, id, dummy_inner());
        let out = p.handle(&propose, &mut s).unwrap();
        assert_eq!(out.derived_commands.len(), 1);
    }

    #[test]
    fn non_owner_cannot_propose_or_confirm() {
        let p = MultisigPrism::new();
        let mut s = StateTree::new();
        let id = acc_id(&p.handle(&create(&["a", "b"], 2), &mut s).unwrap());
        let bad = propose_command(
            PartyId::new("intruder"),
            Visibility::Public,
            0,
            id,
            dummy_inner(),
        );
        assert!(p.handle(&bad, &mut s).is_err());
    }

    #[test]
    fn double_confirm_does_not_double_count() {
        let p = MultisigPrism::new();
        let mut s = StateTree::new();
        let id = acc_id(&p.handle(&create(&["a", "b", "c"], 3), &mut s).unwrap());
        p.handle(
            &propose_command(PartyId::new("a"), Visibility::Public, 1, id, dummy_inner()),
            &mut s,
        )
        .unwrap();
        let again = confirm_command(PartyId::new("a"), Visibility::Public, 2, id, 0);
        let out = p.handle(&again, &mut s).unwrap();
        assert!(
            out.derived_commands.is_empty(),
            "the same owner confirming twice must not reach threshold 3"
        );
    }

    #[test]
    fn rejects_bad_threshold() {
        let p = MultisigPrism::new();
        let mut s = StateTree::new();
        assert!(p.handle(&create(&["a", "b"], 3), &mut s).is_err());
        assert!(p.handle(&create(&["a", "b"], 0), &mut s).is_err());
    }
}
