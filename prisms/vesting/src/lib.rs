use serde::{Deserialize, Serialize};

use prism_token::{credit, debit};
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

const SCHED_PREFIX: &str = "vesting/sched/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Hash,
    pub token_id: Hash,
    pub funder: PartyId,
    pub beneficiary: PartyId,
    #[serde(with = "u128_dec")]
    pub total: u128,
    #[serde(with = "u128_dec")]
    pub released: u128,
    pub start: u64,
    pub cliff: u64,
    pub duration: u64,
    pub revocable: bool,
    pub revoked: bool,
}

impl Schedule {
    pub fn vested(&self, now: u64) -> u128 {
        if self.duration == 0 {
            return self.total;
        }
        if now < self.start.saturating_add(self.cliff) {
            return 0;
        }
        let elapsed = now.saturating_sub(self.start);
        if elapsed >= self.duration {
            return self.total;
        }
        let num = self.total.saturating_mul(elapsed as u128);
        num / (self.duration as u128)
    }

    pub fn releasable(&self, now: u64) -> u128 {
        self.vested(now).saturating_sub(self.released)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum VestingCommand {
    Create {
        token_id: Hash,
        beneficiary: PartyId,
        #[serde(with = "u128_dec")]
        total: u128,
        start: u64,
        cliff: u64,
        duration: u64,
        revocable: bool,
    },
    Release {
        schedule: Hash,
    },
    Revoke {
        schedule: Hash,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VestingEvent {
    Created {
        schedule: Hash,
        beneficiary: PartyId,
        #[serde(with = "u128_dec")]
        total: u128,
    },
    Released {
        schedule: Hash,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Revoked {
        schedule: Hash,
        #[serde(with = "u128_dec")]
        refunded: u128,
    },
}

#[derive(Default)]
pub struct VestingPrism;

impl VestingPrism {
    pub fn new() -> Self {
        VestingPrism
    }

    fn key(id: &Hash) -> String {
        format!("{SCHED_PREFIX}{}", id.to_hex())
    }

    fn now(state: &StateTree) -> u64 {
        state
            .get_json::<u64>("chain/now")
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    fn load(state: &StateTree, id: &Hash) -> Result<Schedule, PrismError> {
        state
            .get_json::<Schedule>(&Self::key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("vesting schedule {}", id.to_hex())))
    }

    fn event(cmd: &Command, payload: VestingEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "vesting".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for VestingPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "vesting",
            description:
                "Time-locked token release schedules (cliff + linear) with optional revoke",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: VestingCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;
        let caller = command.submitter.clone();

        match cmd {
            VestingCommand::Create {
                token_id,
                beneficiary,
                total,
                start,
                cliff,
                duration,
                revocable,
            } => {
                if total == 0 {
                    return Err(PrismError::InvalidPayload("total must be > 0".into()));
                }
                if cliff > duration {
                    return Err(PrismError::InvalidPayload(
                        "cliff must not exceed duration".into(),
                    ));
                }
                let id = Hash::commit(
                    "vesting/id",
                    &[
                        caller.0.as_bytes(),
                        beneficiary.0.as_bytes(),
                        token_id.to_hex().as_bytes(),
                        &start.to_be_bytes(),
                        &total.to_be_bytes(),
                    ],
                );
                if state.contains(&Self::key(&id)) {
                    return Err(PrismError::InvalidPayload("schedule already exists".into()));
                }
                debit(state, &token_id, &caller, total)?;
                let sched = Schedule {
                    id,
                    token_id,
                    funder: caller.clone(),
                    beneficiary: beneficiary.clone(),
                    total,
                    released: 0,
                    start,
                    cliff,
                    duration,
                    revocable,
                    revoked: false,
                };
                state
                    .put_json(Self::key(&id), &sched)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        VestingEvent::Created {
                            schedule: id,
                            beneficiary,
                            total,
                        },
                    ),
                    4_000,
                ))
            }

            VestingCommand::Release { schedule } => {
                let mut sched = Self::load(state, &schedule)?;
                if sched.revoked {
                    return Err(PrismError::InvalidPayload("schedule revoked".into()));
                }
                let now = Self::now(state);
                let amount = sched.releasable(now);
                if amount == 0 {
                    return Err(PrismError::LimitExceeded("nothing vested yet".into()));
                }
                sched.released = sched.released.saturating_add(amount);
                credit(state, &sched.token_id, &sched.beneficiary, amount)?;
                let to = sched.beneficiary.clone();
                state
                    .put_json(Self::key(&schedule), &sched)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        VestingEvent::Released {
                            schedule,
                            to,
                            amount,
                        },
                    ),
                    1_500,
                ))
            }

            VestingCommand::Revoke { schedule } => {
                let mut sched = Self::load(state, &schedule)?;
                if !sched.revocable {
                    return Err(PrismError::Unauthorized("schedule is not revocable".into()));
                }
                if sched.funder != caller {
                    return Err(PrismError::Unauthorized("only funder can revoke".into()));
                }
                if sched.revoked {
                    return Err(PrismError::InvalidPayload("already revoked".into()));
                }
                let now = Self::now(state);
                let vested = sched.vested(now);
                let owed = vested.saturating_sub(sched.released);
                if owed > 0 {
                    credit(state, &sched.token_id, &sched.beneficiary, owed)?;
                    sched.released = sched.released.saturating_add(owed);
                }
                let refund = sched.total.saturating_sub(sched.released);
                if refund > 0 {
                    credit(state, &sched.token_id, &sched.funder, refund)?;
                }
                sched.revoked = true;
                state
                    .put_json(Self::key(&schedule), &sched)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        VestingEvent::Revoked {
                            schedule,
                            refunded: refund,
                        },
                    ),
                    1_500,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<VestingCommand>(&command.payload) {
            Ok(VestingCommand::Create { .. }) => 4_000,
            Ok(_) => 1_500,
            Err(_) => 1_000,
        }
    }
}

pub fn schedule_of(state: &StateTree, id: &Hash) -> Option<Schedule> {
    state.get_json(&VestingPrism::key(id)).ok().flatten()
}

#[allow(clippy::too_many_arguments)]
pub fn create_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    token_id: Hash,
    beneficiary: PartyId,
    total: u128,
    start: u64,
    cliff: u64,
    duration: u64,
    revocable: bool,
) -> Command {
    let payload = serde_json::to_vec(&VestingCommand::Create {
        token_id,
        beneficiary,
        total,
        start,
        cliff,
        duration,
        revocable,
    })
    .unwrap_or_default();
    Command {
        prism: "vesting".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn release_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    schedule: Hash,
) -> Command {
    let payload = serde_json::to_vec(&VestingCommand::Release { schedule }).unwrap_or_default();
    Command {
        prism: "vesting".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_token::{balance_of, seed_native_token};

    fn setup() -> (StateTree, Hash) {
        let mut s = StateTree::new();
        let id = seed_native_token(
            &mut s,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("funder"), 1_000_000)],
        )
        .unwrap();
        (s, id)
    }

    fn set_now(s: &mut StateTree, now: u64) {
        s.put_json("chain/now", &now).unwrap();
    }

    fn create(token: Hash, cliff: u64, duration: u64, revocable: bool) -> Command {
        create_command(
            PartyId::new("funder"),
            Visibility::Public,
            0,
            token,
            PartyId::new("team"),
            120_000,
            0,
            cliff,
            duration,
            revocable,
        )
    }

    fn sched_id(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<VestingEvent>(&out.events[0].payload).unwrap() {
            VestingEvent::Created { schedule, .. } => schedule,
            _ => panic!("expected Created"),
        }
    }

    #[test]
    fn linear_release_after_cliff() {
        let p = VestingPrism::new();
        let (mut s, token) = setup();
        set_now(&mut s, 0);
        let id = sched_id(&p.handle(&create(token, 100, 1_000, false), &mut s).unwrap());
        assert_eq!(balance_of(&s, &token, &PartyId::new("funder")), 880_000);

        set_now(&mut s, 50);
        assert!(
            p.handle(
                &release_command(PartyId::new("team"), Visibility::Public, 0, id),
                &mut s
            )
            .is_err(),
            "before cliff nothing is releasable"
        );

        set_now(&mut s, 500);
        p.handle(
            &release_command(PartyId::new("team"), Visibility::Public, 1, id),
            &mut s,
        )
        .unwrap();
        assert_eq!(balance_of(&s, &token, &PartyId::new("team")), 60_000);

        set_now(&mut s, 5_000);
        p.handle(
            &release_command(PartyId::new("team"), Visibility::Public, 2, id),
            &mut s,
        )
        .unwrap();
        assert_eq!(balance_of(&s, &token, &PartyId::new("team")), 120_000);
    }

    #[test]
    fn revoke_refunds_unvested_and_pays_vested() {
        let p = VestingPrism::new();
        let (mut s, token) = setup();
        set_now(&mut s, 0);
        let id = sched_id(&p.handle(&create(token, 0, 1_000, true), &mut s).unwrap());
        set_now(&mut s, 250);
        let rev = Command {
            prism: "vesting".into(),
            submitter: PartyId::new("funder"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&VestingCommand::Revoke { schedule: id }).unwrap(),
            nonce: 1,
        };
        p.handle(&rev, &mut s).unwrap();
        assert_eq!(balance_of(&s, &token, &PartyId::new("team")), 30_000);
        assert_eq!(balance_of(&s, &token, &PartyId::new("funder")), 970_000);
    }

    #[test]
    fn non_revocable_cannot_be_revoked() {
        let p = VestingPrism::new();
        let (mut s, token) = setup();
        set_now(&mut s, 0);
        let id = sched_id(&p.handle(&create(token, 0, 1_000, false), &mut s).unwrap());
        let rev = Command {
            prism: "vesting".into(),
            submitter: PartyId::new("funder"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&VestingCommand::Revoke { schedule: id }).unwrap(),
            nonce: 1,
        };
        assert!(p.handle(&rev, &mut s).is_err());
    }

    #[test]
    fn cannot_create_with_insufficient_balance() {
        let p = VestingPrism::new();
        let mut s = StateTree::new();
        let token = seed_native_token(
            &mut s,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("funder"), 10)],
        )
        .unwrap();
        set_now(&mut s, 0);
        assert!(p.handle(&create(token, 0, 1_000, false), &mut s).is_err());
    }
}
