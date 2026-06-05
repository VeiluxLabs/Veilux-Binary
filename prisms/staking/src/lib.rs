use serde::{Deserialize, Serialize};

use prism_token::{burn_from, move_balance, native_token_id};
use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};
use veilux_veil::verify_bytes;

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

const STAKE_PREFIX: &str = "staking/stake/";
const DELEG_PREFIX: &str = "staking/delegation/";
const PROPOSAL_PREFIX: &str = "gov/proposal/";
const VOTE_PREFIX: &str = "gov/vote/";
const SLASH_PREFIX: &str = "staking/slashed/";
const REWARD_ACC_KEY: &str = "staking/reward_acc";
const REWARD_DEBT_PREFIX: &str = "staking/reward_debt/";
const TOTAL_STAKE_KEY: &str = "staking/total_stake";

const ACC_SCALE: u128 = 1_000_000_000_000;

pub const DEFAULT_SLASH_BPS: u16 = 2_000;

pub fn escrow_account() -> PartyId {
    PartyId::new("staking/escrow")
}

pub fn reward_pool_account() -> PartyId {
    PartyId::new("staking/rewards")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Active,
    Passed,
    Rejected,
    Executed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StakeRecord {
    pub owner: PartyId,
    #[serde(with = "u128_dec")]
    pub self_bonded: u128,
    #[serde(with = "u128_dec")]
    pub delegated_in: u128,
}

impl StakeRecord {
    fn empty(owner: PartyId) -> Self {
        StakeRecord {
            owner,
            self_bonded: 0,
            delegated_in: 0,
        }
    }

    pub fn voting_power(&self) -> u128 {
        self.self_bonded + self.delegated_in
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Hash,
    pub proposer: PartyId,
    pub title: String,
    pub description: String,
    pub status: ProposalStatus,
    #[serde(with = "u128_dec")]
    pub yes_power: u128,
    #[serde(with = "u128_dec")]
    pub no_power: u128,
    pub deadline_height: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquivocationProof {
    pub offender: PartyId,
    pub public_key: String,
    pub message_a: String,
    pub signature_a: String,
    pub message_b: String,
    pub signature_b: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StakingCommand {
    Stake {
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Unstake {
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Delegate {
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Undelegate {
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Propose {
        title: String,
        description: String,
        voting_period: u64,
    },
    Vote {
        proposal_id: Hash,
        approve: bool,
    },
    Finalize {
        proposal_id: Hash,
    },
    Slash {
        proof: EquivocationProof,
    },
    FundRewards {
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    ClaimRewards,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StakingEvent {
    Staked {
        who: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
        #[serde(with = "u128_dec")]
        total_self: u128,
    },
    Unstaked {
        who: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
        #[serde(with = "u128_dec")]
        total_self: u128,
    },
    Delegated {
        delegator: PartyId,
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Undelegated {
        delegator: PartyId,
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    ProposalOpened {
        proposal_id: Hash,
        proposer: PartyId,
        title: String,
        deadline_height: u64,
    },
    Voted {
        proposal_id: Hash,
        voter: PartyId,
        approve: bool,
        #[serde(with = "u128_dec")]
        power: u128,
    },
    ProposalFinalized {
        proposal_id: Hash,
        status: ProposalStatus,
        #[serde(with = "u128_dec")]
        yes_power: u128,
        #[serde(with = "u128_dec")]
        no_power: u128,
    },
    Slashed {
        offender: PartyId,
        #[serde(with = "u128_dec")]
        burned: u128,
        #[serde(with = "u128_dec")]
        remaining_self: u128,
    },
    RewardsFunded {
        funder: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    RewardsClaimed {
        who: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
}

fn split_height(payload: &[u8]) -> (&[u8], u64) {
    if payload.len() < 8 {
        return (payload, 0);
    }
    let (body, tail) = payload.split_at(payload.len() - 8);
    let mut h = [0u8; 8];
    h.copy_from_slice(tail);
    (body, u64::from_le_bytes(h))
}

#[derive(Default)]
pub struct StakingPrism;

impl StakingPrism {
    pub fn new() -> Self {
        StakingPrism
    }

    fn stake_key(p: &PartyId) -> String {
        format!("{STAKE_PREFIX}{}", p.0)
    }

    fn deleg_key(delegator: &PartyId, validator: &PartyId) -> String {
        format!("{DELEG_PREFIX}{}/{}", delegator.0, validator.0)
    }

    fn proposal_key(id: &Hash) -> String {
        format!("{PROPOSAL_PREFIX}{}", id.to_hex())
    }

    fn vote_key(id: &Hash, voter: &PartyId) -> String {
        format!("{VOTE_PREFIX}{}/{}", id.to_hex(), voter.0)
    }

    fn slash_key(offence: &Hash) -> String {
        format!("{SLASH_PREFIX}{}", offence.to_hex())
    }

    fn reward_debt_key(who: &PartyId) -> String {
        format!("{REWARD_DEBT_PREFIX}{}", who.0)
    }

    fn locked_key(who: &PartyId) -> String {
        format!("staking/locked/{}", who.0)
    }

    fn locked_of(state: &StateTree, who: &PartyId) -> u128 {
        Self::read_u128(state, &Self::locked_key(who))
    }

    fn harvest_pending(state: &mut StateTree, who: &PartyId) -> Result<u128, PrismError> {
        let locked = Self::locked_of(state, who);
        let pending = Self::pending_reward(state, who, locked);
        if pending > 0 {
            move_balance(
                state,
                &native_token_id(),
                &reward_pool_account(),
                who,
                pending,
            )?;
        }
        Ok(pending)
    }

    fn update_lock(state: &mut StateTree, who: &PartyId, delta: i128) -> Result<(), PrismError> {
        Self::harvest_pending(state, who)?;
        let cur = Self::locked_of(state, who) as i128;
        let next = (cur + delta).max(0) as u128;
        Self::write_u128(state, &Self::locked_key(who), next)?;
        Self::add_total_stake(state, delta)?;
        Self::settle_reward_debt(state, who, next)?;
        Ok(())
    }

    fn read_u128(state: &StateTree, key: &str) -> u128 {
        state
            .get_json::<String>(key)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn write_u128(state: &mut StateTree, key: &str, v: u128) -> Result<(), PrismError> {
        state
            .put_json(key, &v.to_string())
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn total_stake(state: &StateTree) -> u128 {
        Self::read_u128(state, TOTAL_STAKE_KEY)
    }

    fn add_total_stake(state: &mut StateTree, delta: i128) -> Result<(), PrismError> {
        let cur = Self::total_stake(state) as i128;
        let next = (cur + delta).max(0) as u128;
        Self::write_u128(state, TOTAL_STAKE_KEY, next)
    }

    fn reward_acc(state: &StateTree) -> u128 {
        Self::read_u128(state, REWARD_ACC_KEY)
    }

    fn pending_reward(state: &StateTree, who: &PartyId, power: u128) -> u128 {
        let acc = Self::reward_acc(state);
        let entitled = power.saturating_mul(acc) / ACC_SCALE;
        let debt = Self::read_u128(state, &Self::reward_debt_key(who));
        entitled.saturating_sub(debt)
    }

    fn settle_reward_debt(
        state: &mut StateTree,
        who: &PartyId,
        power: u128,
    ) -> Result<(), PrismError> {
        let acc = Self::reward_acc(state);
        let entitled = power.saturating_mul(acc) / ACC_SCALE;
        Self::write_u128(state, &Self::reward_debt_key(who), entitled)
    }

    pub fn stake_of(state: &StateTree, who: &PartyId) -> StakeRecord {
        state
            .get_json::<StakeRecord>(&Self::stake_key(who))
            .ok()
            .flatten()
            .unwrap_or_else(|| StakeRecord::empty(who.clone()))
    }

    fn save_stake(state: &mut StateTree, rec: &StakeRecord) -> Result<(), PrismError> {
        state
            .put_json(Self::stake_key(&rec.owner), rec)
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn delegation(state: &StateTree, delegator: &PartyId, validator: &PartyId) -> u128 {
        state
            .get_json::<String>(&Self::deleg_key(delegator, validator))
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn set_delegation(
        state: &mut StateTree,
        delegator: &PartyId,
        validator: &PartyId,
        v: u128,
    ) -> Result<(), PrismError> {
        state
            .put_json(Self::deleg_key(delegator, validator), &v.to_string())
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn event(cmd: &Command, payload: StakingEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "staking".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for StakingPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "staking",
            description: "Stake native LUX, delegate, and vote on stake-weighted governance",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let (body, height) = split_height(&command.payload);
        let cmd: StakingCommand =
            serde_json::from_slice(body).map_err(|e| PrismError::InvalidPayload(e.to_string()))?;
        let token = native_token_id();
        let me = command.submitter.clone();

        match cmd {
            StakingCommand::Stake { amount } => {
                if amount == 0 {
                    return Err(PrismError::InvalidPayload("amount must be > 0".into()));
                }
                move_balance(state, &token, &me, &escrow_account(), amount)?;
                let mut rec = Self::stake_of(state, &me);
                rec.self_bonded += amount;
                Self::save_stake(state, &rec)?;
                Self::update_lock(state, &me, amount as i128)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Staked {
                            who: me,
                            amount,
                            total_self: rec.self_bonded,
                        },
                    ),
                    3_000,
                ))
            }

            StakingCommand::Unstake { amount } => {
                let mut rec = Self::stake_of(state, &me);
                if rec.self_bonded < amount {
                    return Err(PrismError::LimitExceeded(
                        "not enough self-bonded stake".into(),
                    ));
                }
                move_balance(state, &token, &escrow_account(), &me, amount)?;
                rec.self_bonded -= amount;
                Self::save_stake(state, &rec)?;
                Self::update_lock(state, &me, -(amount as i128))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Unstaked {
                            who: me,
                            amount,
                            total_self: rec.self_bonded,
                        },
                    ),
                    3_000,
                ))
            }

            StakingCommand::Delegate { validator, amount } => {
                if amount == 0 {
                    return Err(PrismError::InvalidPayload("amount must be > 0".into()));
                }
                move_balance(state, &token, &me, &escrow_account(), amount)?;
                let cur = Self::delegation(state, &me, &validator);
                Self::set_delegation(state, &me, &validator, cur + amount)?;
                let mut vrec = Self::stake_of(state, &validator);
                vrec.delegated_in += amount;
                Self::save_stake(state, &vrec)?;
                Self::update_lock(state, &me, amount as i128)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Delegated {
                            delegator: me,
                            validator,
                            amount,
                        },
                    ),
                    3_500,
                ))
            }

            StakingCommand::Undelegate { validator, amount } => {
                let cur = Self::delegation(state, &me, &validator);
                if cur < amount {
                    return Err(PrismError::LimitExceeded("not enough delegated".into()));
                }
                move_balance(state, &token, &escrow_account(), &me, amount)?;
                Self::set_delegation(state, &me, &validator, cur - amount)?;
                let mut vrec = Self::stake_of(state, &validator);
                vrec.delegated_in = vrec.delegated_in.saturating_sub(amount);
                Self::save_stake(state, &vrec)?;
                Self::update_lock(state, &me, -(amount as i128))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Undelegated {
                            delegator: me,
                            validator,
                            amount,
                        },
                    ),
                    3_500,
                ))
            }

            StakingCommand::Propose {
                title,
                description,
                voting_period,
            } => {
                let rec = Self::stake_of(state, &me);
                if rec.self_bonded == 0 {
                    return Err(PrismError::Unauthorized(
                        "only bonded stakers can open proposals".into(),
                    ));
                }
                let id = Hash::commit(
                    "gov/proposal",
                    &[
                        me.0.as_bytes(),
                        title.as_bytes(),
                        &command.nonce.to_le_bytes(),
                    ],
                );
                if state.contains(&Self::proposal_key(&id)) {
                    return Err(PrismError::InvalidPayload("proposal already exists".into()));
                }
                let deadline = height + voting_period.max(1);
                let proposal = Proposal {
                    id,
                    proposer: me.clone(),
                    title: title.clone(),
                    description,
                    status: ProposalStatus::Active,
                    yes_power: 0,
                    no_power: 0,
                    deadline_height: deadline,
                };
                state
                    .put_json(Self::proposal_key(&id), &proposal)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::ProposalOpened {
                            proposal_id: id,
                            proposer: me,
                            title,
                            deadline_height: deadline,
                        },
                    ),
                    5_000,
                ))
            }

            StakingCommand::Vote {
                proposal_id,
                approve,
            } => {
                let mut proposal: Proposal = state
                    .get_json(&Self::proposal_key(&proposal_id))
                    .map_err(|e| PrismError::Internal(e.to_string()))?
                    .ok_or_else(|| PrismError::NotFound("proposal".into()))?;
                if proposal.status != ProposalStatus::Active {
                    return Err(PrismError::InvalidPayload("proposal not active".into()));
                }
                if height > proposal.deadline_height {
                    return Err(PrismError::InvalidPayload("voting period ended".into()));
                }
                if state.contains(&Self::vote_key(&proposal_id, &me)) {
                    return Err(PrismError::InvalidPayload("already voted".into()));
                }
                let power = Self::stake_of(state, &me).voting_power();
                if power == 0 {
                    return Err(PrismError::Unauthorized("no voting power".into()));
                }
                if approve {
                    proposal.yes_power += power;
                } else {
                    proposal.no_power += power;
                }
                state
                    .put_json(Self::vote_key(&proposal_id, &me), &approve)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                state
                    .put_json(Self::proposal_key(&proposal_id), &proposal)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Voted {
                            proposal_id,
                            voter: me,
                            approve,
                            power,
                        },
                    ),
                    2_000,
                ))
            }

            StakingCommand::Finalize { proposal_id } => {
                let mut proposal: Proposal = state
                    .get_json(&Self::proposal_key(&proposal_id))
                    .map_err(|e| PrismError::Internal(e.to_string()))?
                    .ok_or_else(|| PrismError::NotFound("proposal".into()))?;
                if proposal.status != ProposalStatus::Active {
                    return Err(PrismError::InvalidPayload("already finalized".into()));
                }
                if height <= proposal.deadline_height {
                    return Err(PrismError::InvalidPayload("voting period not over".into()));
                }
                proposal.status = if proposal.yes_power > proposal.no_power {
                    ProposalStatus::Passed
                } else {
                    ProposalStatus::Rejected
                };
                state
                    .put_json(Self::proposal_key(&proposal_id), &proposal)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::ProposalFinalized {
                            proposal_id,
                            status: proposal.status,
                            yes_power: proposal.yes_power,
                            no_power: proposal.no_power,
                        },
                    ),
                    2_500,
                ))
            }

            StakingCommand::Slash { proof } => {
                let pk = hex::decode(proof.public_key.trim_start_matches("0x"))
                    .map_err(|_| PrismError::InvalidPayload("bad public key hex".into()))?;
                let ma = hex::decode(proof.message_a.trim_start_matches("0x"))
                    .map_err(|_| PrismError::InvalidPayload("bad message_a hex".into()))?;
                let mb = hex::decode(proof.message_b.trim_start_matches("0x"))
                    .map_err(|_| PrismError::InvalidPayload("bad message_b hex".into()))?;
                let sa = hex::decode(proof.signature_a.trim_start_matches("0x"))
                    .map_err(|_| PrismError::InvalidPayload("bad signature_a hex".into()))?;
                let sb = hex::decode(proof.signature_b.trim_start_matches("0x"))
                    .map_err(|_| PrismError::InvalidPayload("bad signature_b hex".into()))?;

                if ma == mb {
                    return Err(PrismError::InvalidPayload(
                        "messages are identical — not equivocation".into(),
                    ));
                }
                verify_bytes(&pk, &ma, &sa)
                    .map_err(|_| PrismError::Unauthorized("signature_a invalid".into()))?;
                verify_bytes(&pk, &mb, &sb)
                    .map_err(|_| PrismError::Unauthorized("signature_b invalid".into()))?;

                let (lo, hi) = if ma <= mb { (&ma, &mb) } else { (&mb, &ma) };
                let offence = Hash::commit(
                    "staking/equivocation",
                    &[proof.offender.0.as_bytes(), lo, hi],
                );
                if state.contains(&Self::slash_key(&offence)) {
                    return Err(PrismError::InvalidPayload("offence already slashed".into()));
                }

                let mut rec = Self::stake_of(state, &proof.offender);
                let burned = rec.self_bonded * (DEFAULT_SLASH_BPS as u128) / 10_000;
                if burned > 0 {
                    burn_from(state, &native_token_id(), &escrow_account(), burned)
                        .map_err(|_| PrismError::Internal("slash burn failed".into()))?;
                    rec.self_bonded -= burned;
                    Self::save_stake(state, &rec)?;
                    Self::update_lock(state, &proof.offender, -(burned as i128))?;
                }
                state
                    .put_json(Self::slash_key(&offence), &true)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::Slashed {
                            offender: proof.offender,
                            burned,
                            remaining_self: rec.self_bonded,
                        },
                    ),
                    7_000,
                ))
            }

            StakingCommand::FundRewards { amount } => {
                if amount == 0 {
                    return Err(PrismError::InvalidPayload("amount must be > 0".into()));
                }
                let total = Self::total_stake(state);
                if total == 0 {
                    return Err(PrismError::InvalidPayload("no stake to reward yet".into()));
                }
                move_balance(state, &token, &me, &reward_pool_account(), amount)?;
                let acc = Self::reward_acc(state);
                let added = amount.saturating_mul(ACC_SCALE) / total;
                Self::write_u128(state, REWARD_ACC_KEY, acc.saturating_add(added))?;
                Ok(PrismOutput::single(
                    Self::event(command, StakingEvent::RewardsFunded { funder: me, amount }),
                    2_000,
                ))
            }

            StakingCommand::ClaimRewards => {
                let claimed = Self::harvest_pending(state, &me)?;
                let locked = Self::locked_of(state, &me);
                Self::settle_reward_debt(state, &me, locked)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        StakingEvent::RewardsClaimed {
                            who: me,
                            amount: claimed,
                        },
                    ),
                    2_500,
                ))
            }
        }
    }

    fn estimate(&self, _command: &Command, _state: &StateTree) -> u64 {
        3_000
    }
}

pub fn staking_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    height: u64,
    cmd: &StakingCommand,
) -> Command {
    let mut payload = serde_json::to_vec(cmd).unwrap_or_default();
    payload.extend_from_slice(&height.to_le_bytes());
    Command {
        prism: "staking".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn voting_power_of(state: &StateTree, who: &PartyId) -> u128 {
    StakingPrism::stake_of(state, who).voting_power()
}

pub fn pending_rewards_of(state: &StateTree, who: &PartyId) -> u128 {
    let locked = StakingPrism::locked_of(state, who);
    StakingPrism::pending_reward(state, who, locked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_token::{balance_of, seed_native_token};

    fn setup() -> (StakingPrism, StateTree, Hash) {
        let mut state = StateTree::new();
        let id = seed_native_token(
            &mut state,
            "Veilux",
            "LUX",
            18,
            &PartyId::new("treasury"),
            &[
                (PartyId::new("alice"), 1_000),
                (PartyId::new("bob"), 1_000),
                (PartyId::new("treasury"), 1_000),
            ],
        )
        .unwrap();
        (StakingPrism::new(), state, id)
    }

    #[test]
    fn stake_and_unstake_roundtrips_balance() {
        let (p, mut s, token) = setup();
        let stake = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 400 },
        );
        p.handle(&stake, &mut s).unwrap();
        assert_eq!(balance_of(&s, &token, &PartyId::new("alice")), 600);
        assert_eq!(voting_power_of(&s, &PartyId::new("alice")), 400);

        let unstake = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            2,
            &StakingCommand::Unstake { amount: 150 },
        );
        p.handle(&unstake, &mut s).unwrap();
        assert_eq!(balance_of(&s, &token, &PartyId::new("alice")), 750);
        assert_eq!(voting_power_of(&s, &PartyId::new("alice")), 250);
    }

    #[test]
    fn cannot_stake_more_than_balance() {
        let (p, mut s, _) = setup();
        let stake = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 5_000 },
        );
        assert!(p.handle(&stake, &mut s).is_err());
    }

    #[test]
    fn delegation_boosts_validator_power() {
        let (p, mut s, _) = setup();
        let d = staking_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Delegate {
                validator: PartyId::new("alice"),
                amount: 300,
            },
        );
        p.handle(&d, &mut s).unwrap();
        assert_eq!(voting_power_of(&s, &PartyId::new("alice")), 300);
    }

    #[test]
    fn governance_proposal_passes_by_stake_weight() {
        let (p, mut s, _) = setup();
        for (who, n) in [("alice", 0u64), ("bob", 0u64)] {
            let stake = staking_command(
                PartyId::new(who),
                Visibility::Public,
                n,
                1,
                &StakingCommand::Stake { amount: 500 },
            );
            p.handle(&stake, &mut s).unwrap();
        }

        let propose = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            1,
            &StakingCommand::Propose {
                title: "raise block size".into(),
                description: "double max commands".into(),
                voting_period: 5,
            },
        );
        let out = p.handle(&propose, &mut s).unwrap();
        let pid = match serde_json::from_slice::<StakingEvent>(&out.events[0].payload).unwrap() {
            StakingEvent::ProposalOpened { proposal_id, .. } => proposal_id,
            _ => panic!("expected ProposalOpened"),
        };

        for (who, n) in [("alice", 2u64), ("bob", 1u64)] {
            let vote = staking_command(
                PartyId::new(who),
                Visibility::Public,
                n,
                2,
                &StakingCommand::Vote {
                    proposal_id: pid,
                    approve: who == "alice",
                },
            );
            p.handle(&vote, &mut s).unwrap();
        }

        let finalize = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            3,
            10,
            &StakingCommand::Finalize { proposal_id: pid },
        );
        let out = p.handle(&finalize, &mut s).unwrap();
        match serde_json::from_slice::<StakingEvent>(&out.events[0].payload).unwrap() {
            StakingEvent::ProposalFinalized { status, .. } => {
                assert!(matches!(
                    status,
                    ProposalStatus::Passed | ProposalStatus::Rejected
                ));
            }
            _ => panic!("expected ProposalFinalized"),
        }
    }

    #[test]
    fn double_vote_is_rejected() {
        let (p, mut s, _) = setup();
        let stake = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 500 },
        );
        p.handle(&stake, &mut s).unwrap();
        let propose = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            1,
            &StakingCommand::Propose {
                title: "t".into(),
                description: "d".into(),
                voting_period: 5,
            },
        );
        let out = p.handle(&propose, &mut s).unwrap();
        let pid = match serde_json::from_slice::<StakingEvent>(&out.events[0].payload).unwrap() {
            StakingEvent::ProposalOpened { proposal_id, .. } => proposal_id,
            _ => unreachable!(),
        };
        let vote = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            2,
            2,
            &StakingCommand::Vote {
                proposal_id: pid,
                approve: true,
            },
        );
        p.handle(&vote, &mut s).unwrap();
        let vote2 = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            3,
            2,
            &StakingCommand::Vote {
                proposal_id: pid,
                approve: false,
            },
        );
        assert!(p.handle(&vote2, &mut s).is_err());
    }

    #[test]
    fn equivocation_evidence_slashes_offender() {
        use veilux_veil::PartyIdentity;
        let (p, mut s, _) = setup();

        let stake = staking_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 1_000 },
        );
        p.handle(&stake, &mut s).unwrap();
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 1_000);

        let bob_key = PartyIdentity::from_seed("bob", &[7u8; 32]);
        let msg_a = b"vote-for-block-A".to_vec();
        let msg_b = b"vote-for-block-B".to_vec();
        let proof = EquivocationProof {
            offender: PartyId::new("bob"),
            public_key: hex::encode(bob_key.public_key()),
            message_a: hex::encode(&msg_a),
            signature_a: hex::encode(bob_key.sign_bytes(&msg_a)),
            message_b: hex::encode(&msg_b),
            signature_b: hex::encode(bob_key.sign_bytes(&msg_b)),
        };

        let slash = staking_command(
            PartyId::new("watchdog"),
            Visibility::Public,
            0,
            3,
            &StakingCommand::Slash {
                proof: proof.clone(),
            },
        );
        let out = p.handle(&slash, &mut s).unwrap();
        match serde_json::from_slice::<StakingEvent>(&out.events[0].payload).unwrap() {
            StakingEvent::Slashed {
                burned,
                remaining_self,
                ..
            } => {
                assert_eq!(burned, 200);
                assert_eq!(remaining_self, 800);
            }
            _ => panic!("expected Slashed"),
        }
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 800);

        let slash2 = staking_command(
            PartyId::new("watchdog"),
            Visibility::Public,
            1,
            4,
            &StakingCommand::Slash { proof },
        );
        assert!(p.handle(&slash2, &mut s).is_err());
    }

    #[test]
    fn forged_equivocation_is_rejected() {
        use veilux_veil::PartyIdentity;
        let (p, mut s, _) = setup();
        let stake = staking_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 1_000 },
        );
        p.handle(&stake, &mut s).unwrap();

        let attacker = PartyIdentity::from_seed("attacker", &[9u8; 32]);
        let proof = EquivocationProof {
            offender: PartyId::new("bob"),
            public_key: hex::encode(attacker.public_key()),
            message_a: hex::encode(b"A"),
            signature_a: hex::encode(attacker.sign_bytes(b"A")),
            message_b: hex::encode(b"B"),
            signature_b: hex::encode(attacker.sign_bytes(b"WRONG")),
        };
        let slash = staking_command(
            PartyId::new("watchdog"),
            Visibility::Public,
            0,
            3,
            &StakingCommand::Slash { proof },
        );
        assert!(p.handle(&slash, &mut s).is_err());
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 1_000);
    }

    #[test]
    fn rewards_distribute_proportionally_to_stake() {
        let (p, mut s, token) = setup();

        let stake = |who: &str, amount: u128, nonce: u64| {
            staking_command(
                PartyId::new(who),
                Visibility::Public,
                nonce,
                1,
                &StakingCommand::Stake { amount },
            )
        };
        p.handle(&stake("alice", 300, 0), &mut s).unwrap();
        p.handle(&stake("bob", 100, 0), &mut s).unwrap();

        let treasury_before = balance_of(&s, &token, &PartyId::new("treasury"));
        let fund = staking_command(
            PartyId::new("treasury"),
            Visibility::Public,
            0,
            2,
            &StakingCommand::FundRewards { amount: 400 },
        );
        p.handle(&fund, &mut s).unwrap();
        assert_eq!(
            balance_of(&s, &token, &PartyId::new("treasury")),
            treasury_before - 400
        );

        assert_eq!(pending_rewards_of(&s, &PartyId::new("alice")), 300);
        assert_eq!(pending_rewards_of(&s, &PartyId::new("bob")), 100);

        let alice_before = balance_of(&s, &token, &PartyId::new("alice"));
        let claim = staking_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            3,
            &StakingCommand::ClaimRewards,
        );
        p.handle(&claim, &mut s).unwrap();
        assert_eq!(
            balance_of(&s, &token, &PartyId::new("alice")),
            alice_before + 300
        );
        assert_eq!(pending_rewards_of(&s, &PartyId::new("alice")), 0);
        assert_eq!(pending_rewards_of(&s, &PartyId::new("bob")), 100);
    }

    #[test]
    fn staking_after_funding_does_not_claim_past_rewards() {
        let (p, mut s, _) = setup();
        p.handle(
            &staking_command(
                PartyId::new("alice"),
                Visibility::Public,
                0,
                1,
                &StakingCommand::Stake { amount: 100 },
            ),
            &mut s,
        )
        .unwrap();
        p.handle(
            &staking_command(
                PartyId::new("treasury"),
                Visibility::Public,
                0,
                2,
                &StakingCommand::FundRewards { amount: 100 },
            ),
            &mut s,
        )
        .unwrap();
        p.handle(
            &staking_command(
                PartyId::new("bob"),
                Visibility::Public,
                0,
                3,
                &StakingCommand::Stake { amount: 100 },
            ),
            &mut s,
        )
        .unwrap();
        assert_eq!(pending_rewards_of(&s, &PartyId::new("alice")), 100);
        assert_eq!(pending_rewards_of(&s, &PartyId::new("bob")), 0);
    }

    #[test]
    fn slashed_stake_stops_earning_rewards() {
        use veilux_veil::PartyIdentity;
        let (p, mut s, _) = setup();

        let stake = |who: &str, amount: u128| {
            staking_command(
                PartyId::new(who),
                Visibility::Public,
                0,
                1,
                &StakingCommand::Stake { amount },
            )
        };
        p.handle(&stake("alice", 500), &mut s).unwrap();
        p.handle(&stake("bob", 500), &mut s).unwrap();

        let bob_key = PartyIdentity::from_seed("bob", &[7u8; 32]);
        let proof = EquivocationProof {
            offender: PartyId::new("bob"),
            public_key: hex::encode(bob_key.public_key()),
            message_a: hex::encode(b"A"),
            signature_a: hex::encode(bob_key.sign_bytes(b"A")),
            message_b: hex::encode(b"B"),
            signature_b: hex::encode(bob_key.sign_bytes(b"B")),
        };
        p.handle(
            &staking_command(
                PartyId::new("watchdog"),
                Visibility::Public,
                0,
                2,
                &StakingCommand::Slash { proof },
            ),
            &mut s,
        )
        .unwrap();

        p.handle(
            &staking_command(
                PartyId::new("treasury"),
                Visibility::Public,
                0,
                3,
                &StakingCommand::FundRewards { amount: 900 },
            ),
            &mut s,
        )
        .unwrap();

        let alice_share = pending_rewards_of(&s, &PartyId::new("alice"));
        let bob_share = pending_rewards_of(&s, &PartyId::new("bob"));
        assert_eq!(alice_share, 500);
        assert_eq!(bob_share, 400);
        assert_eq!(alice_share + bob_share, 900);
    }
}
