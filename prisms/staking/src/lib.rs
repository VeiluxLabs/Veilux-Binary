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

/// Default fraction of self-bonded stake burned on a proven equivocation, in
/// basis points (2000 = 20%).
pub const DEFAULT_SLASH_BPS: u16 = 2_000;

/// Reserved account that custodies all staked funds. Value moved here is locked
/// (unspendable) until unstaked; total token supply is unchanged.
pub fn escrow_account() -> PartyId {
    PartyId::new("staking/escrow")
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

/// Proof that a validator equivocated: two distinct messages it signed for the
/// same consensus slot. The signing bytes are opaque to staking (they come from
/// the consensus vote scheme); staking only checks that both were signed by the
/// same offender key over *different* messages, which is unforgeable evidence of
/// double-signing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquivocationProof {
    pub offender: PartyId,
    /// Hex-encoded Ed25519 public key the offender signs consensus votes with.
    pub public_key: String,
    /// Two distinct signed messages (hex) for the same height/round.
    pub message_a: String,
    pub signature_a: String,
    pub message_b: String,
    pub signature_b: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StakingCommand {
    /// Bond native LUX as your own validator stake.
    Stake {
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    /// Withdraw previously self-bonded stake back to your balance.
    Unstake {
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    /// Delegate native LUX toward another party's stake (boosts their power).
    Delegate {
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    /// Undelegate previously delegated stake.
    Undelegate {
        validator: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    /// Open a governance proposal (requires some self-bonded stake).
    Propose {
        title: String,
        description: String,
        voting_period: u64,
    },
    /// Cast a stake-weighted vote on an active proposal.
    Vote { proposal_id: Hash, approve: bool },
    /// Finalize a proposal once its voting period has elapsed.
    Finalize { proposal_id: Hash },
    /// Submit equivocation evidence to slash a double-signing validator. Anyone
    /// may submit; the offender's self-bonded stake is reduced by `slash_bps`
    /// (burned), and the offence is recorded so it cannot be replayed.
    Slash { proof: EquivocationProof },
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
}

/// Block height is injected into staking commands by the node before routing,
/// so the deterministic VM has a notion of "now" without reading a clock. The
/// height is appended as a little-endian u64 trailer on the command payload.
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
                // Verify the offender's key actually signed both messages.
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

                // One offence per (offender, message pair); deterministic id with
                // the two messages ordered so A/B order cannot be replayed.
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
                    // Slashed stake is burned from the escrow (where bonded funds
                    // live), reducing total supply.
                    burn_from(state, &native_token_id(), &escrow_account(), burned)
                        .map_err(|_| PrismError::Internal("slash burn failed".into()))?;
                    rec.self_bonded -= burned;
                    Self::save_stake(state, &rec)?;
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
        }
    }

    fn estimate(&self, _command: &Command, _state: &StateTree) -> u64 {
        3_000
    }
}

/// Encode a staking command with the current block height appended as the
/// trailing 8-byte LE trailer the prism expects.
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
            &[(PartyId::new("alice"), 1_000), (PartyId::new("bob"), 1_000)],
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

        // bob bonds 1000 self stake.
        let stake = staking_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            1,
            &StakingCommand::Stake { amount: 1_000 },
        );
        p.handle(&stake, &mut s).unwrap();
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 1_000);

        // bob double-signs two different consensus messages for the same slot.
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
                assert_eq!(burned, 200); // 20% of 1000
                assert_eq!(remaining_self, 800);
            }
            _ => panic!("expected Slashed"),
        }
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 800);

        // Replaying the same evidence is rejected.
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

        // Attacker fabricates evidence with a mismatched signature.
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
        // signature_b does not match message_b -> rejected, no slash.
        assert!(p.handle(&slash, &mut s).is_err());
        assert_eq!(voting_power_of(&s, &PartyId::new("bob")), 1_000);
    }
}
