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

const META_PREFIX: &str = "token/meta/";
const BAL_PREFIX: &str = "token/bal/";
const ALLOW_PREFIX: &str = "token/allow/";

pub fn native_token_id() -> Hash {
    Hash::commit("veilux/native-token", &[])
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenMeta {
    pub token_id: Hash,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(with = "u128_dec")]
    pub total_supply: u128,
    pub owner: PartyId,
    pub mintable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TokenCommand {
    Create {
        name: String,
        symbol: String,
        decimals: u8,
        #[serde(with = "u128_dec")]
        initial_supply: u128,
        mintable: bool,
    },
    Transfer {
        token_id: Hash,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Approve {
        token_id: Hash,
        spender: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    TransferFrom {
        token_id: Hash,
        from: PartyId,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Mint {
        token_id: Hash,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Burn {
        token_id: Hash,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenEvent {
    Created {
        token_id: Hash,
        symbol: String,
        #[serde(with = "u128_dec")]
        total_supply: u128,
    },
    Transfer {
        token_id: Hash,
        from: PartyId,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Approval {
        token_id: Hash,
        owner: PartyId,
        spender: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Mint {
        token_id: Hash,
        to: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
    Burn {
        token_id: Hash,
        from: PartyId,
        #[serde(with = "u128_dec")]
        amount: u128,
    },
}

#[derive(Default)]
pub struct TokenPrism;

impl TokenPrism {
    pub fn new() -> Self {
        TokenPrism
    }

    fn meta_key(id: &Hash) -> String {
        format!("{META_PREFIX}{}", id.to_hex())
    }

    fn bal_key(id: &Hash, who: &PartyId) -> String {
        format!("{BAL_PREFIX}{}/{}", id.to_hex(), who.0)
    }

    fn allow_key(id: &Hash, owner: &PartyId, spender: &PartyId) -> String {
        format!("{ALLOW_PREFIX}{}/{}/{}", id.to_hex(), owner.0, spender.0)
    }

    fn balance(state: &StateTree, id: &Hash, who: &PartyId) -> u128 {
        state
            .get_json::<String>(&Self::bal_key(id, who))
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0)
    }

    fn set_balance(
        state: &mut StateTree,
        id: &Hash,
        who: &PartyId,
        v: u128,
    ) -> Result<(), PrismError> {
        state
            .put_json(Self::bal_key(id, who), &v.to_string())
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn allowance(state: &StateTree, id: &Hash, owner: &PartyId, spender: &PartyId) -> u128 {
        state
            .get_json::<String>(&Self::allow_key(id, owner, spender))
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0)
    }

    fn set_allowance(
        state: &mut StateTree,
        id: &Hash,
        owner: &PartyId,
        spender: &PartyId,
        v: u128,
    ) -> Result<(), PrismError> {
        state
            .put_json(Self::allow_key(id, owner, spender), &v.to_string())
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn load_meta(state: &StateTree, id: &Hash) -> Result<TokenMeta, PrismError> {
        state
            .get_json::<TokenMeta>(&Self::meta_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("token {}", id.to_hex())))
    }

    fn event(cmd: &Command, payload: TokenEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "token".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for TokenPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "token",
            description: "Fungible tokens with transfer, approve, mint and burn",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: TokenCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            TokenCommand::Create {
                name,
                symbol,
                decimals,
                initial_supply,
                mintable,
            } => {
                let token_id = Hash::commit(
                    "token/id",
                    &[
                        command.submitter.0.as_bytes(),
                        symbol.as_bytes(),
                        name.as_bytes(),
                    ],
                );
                if state.contains(&Self::meta_key(&token_id)) {
                    return Err(PrismError::InvalidPayload("token already exists".into()));
                }
                let meta = TokenMeta {
                    token_id,
                    name,
                    symbol: symbol.clone(),
                    decimals,
                    total_supply: initial_supply,
                    owner: command.submitter.clone(),
                    mintable,
                };
                state
                    .put_json(Self::meta_key(&token_id), &meta)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;
                Self::set_balance(state, &token_id, &command.submitter, initial_supply)?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Created {
                            token_id,
                            symbol,
                            total_supply: initial_supply,
                        },
                    ),
                    5_000,
                ))
            }

            TokenCommand::Transfer {
                token_id,
                to,
                amount,
            } => {
                let _ = Self::load_meta(state, &token_id)?;
                let from = command.submitter.clone();
                let from_bal = Self::balance(state, &token_id, &from);
                if from_bal < amount {
                    return Err(PrismError::LimitExceeded("insufficient balance".into()));
                }
                let to_bal = Self::balance(state, &token_id, &to);
                Self::set_balance(state, &token_id, &from, from_bal - amount)?;
                Self::set_balance(
                    state,
                    &token_id,
                    &to,
                    to_bal
                        .checked_add(amount)
                        .ok_or_else(|| PrismError::Internal("balance overflow".into()))?,
                )?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Transfer {
                            token_id,
                            from,
                            to,
                            amount,
                        },
                    ),
                    1_000,
                ))
            }

            TokenCommand::Approve {
                token_id,
                spender,
                amount,
            } => {
                let _ = Self::load_meta(state, &token_id)?;
                let owner = command.submitter.clone();
                Self::set_allowance(state, &token_id, &owner, &spender, amount)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Approval {
                            token_id,
                            owner,
                            spender,
                            amount,
                        },
                    ),
                    800,
                ))
            }

            TokenCommand::TransferFrom {
                token_id,
                from,
                to,
                amount,
            } => {
                let _ = Self::load_meta(state, &token_id)?;
                let spender = command.submitter.clone();
                let allowed = Self::allowance(state, &token_id, &from, &spender);
                if allowed < amount {
                    return Err(PrismError::Unauthorized("allowance exceeded".into()));
                }
                let from_bal = Self::balance(state, &token_id, &from);
                if from_bal < amount {
                    return Err(PrismError::LimitExceeded("insufficient balance".into()));
                }
                let to_bal = Self::balance(state, &token_id, &to);
                Self::set_balance(state, &token_id, &from, from_bal - amount)?;
                Self::set_balance(
                    state,
                    &token_id,
                    &to,
                    to_bal
                        .checked_add(amount)
                        .ok_or_else(|| PrismError::Internal("balance overflow".into()))?,
                )?;
                Self::set_allowance(state, &token_id, &from, &spender, allowed - amount)?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Transfer {
                            token_id,
                            from,
                            to,
                            amount,
                        },
                    ),
                    1_200,
                ))
            }

            TokenCommand::Mint {
                token_id,
                to,
                amount,
            } => {
                let mut meta = Self::load_meta(state, &token_id)?;
                if !meta.mintable {
                    return Err(PrismError::Unauthorized("token not mintable".into()));
                }
                if meta.owner != command.submitter {
                    return Err(PrismError::Unauthorized("only owner can mint".into()));
                }
                let to_bal = Self::balance(state, &token_id, &to);
                Self::set_balance(
                    state,
                    &token_id,
                    &to,
                    to_bal
                        .checked_add(amount)
                        .ok_or_else(|| PrismError::Internal("balance overflow".into()))?,
                )?;
                meta.total_supply = meta
                    .total_supply
                    .checked_add(amount)
                    .ok_or_else(|| PrismError::Internal("supply overflow".into()))?;
                state
                    .put_json(Self::meta_key(&token_id), &meta)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Mint {
                            token_id,
                            to,
                            amount,
                        },
                    ),
                    2_000,
                ))
            }

            TokenCommand::Burn { token_id, amount } => {
                let mut meta = Self::load_meta(state, &token_id)?;
                let from = command.submitter.clone();
                let bal = Self::balance(state, &token_id, &from);
                if bal < amount {
                    return Err(PrismError::LimitExceeded(
                        "insufficient balance to burn".into(),
                    ));
                }
                Self::set_balance(state, &token_id, &from, bal - amount)?;
                meta.total_supply = meta.total_supply.saturating_sub(amount);
                state
                    .put_json(Self::meta_key(&token_id), &meta)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        TokenEvent::Burn {
                            token_id,
                            from,
                            amount,
                        },
                    ),
                    1_500,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<TokenCommand>(&command.payload) {
            Ok(TokenCommand::Create { .. }) => 5_000,
            Ok(_) => 1_200,
            Err(_) => 1_000,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    symbol: &str,
    decimals: u8,
    initial_supply: u128,
    mintable: bool,
) -> Command {
    let payload = serde_json::to_vec(&TokenCommand::Create {
        name: name.to_string(),
        symbol: symbol.to_string(),
        decimals,
        initial_supply,
        mintable,
    })
    .unwrap_or_default();
    Command {
        prism: "token".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn transfer_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    token_id: Hash,
    to: PartyId,
    amount: u128,
) -> Command {
    let payload = serde_json::to_vec(&TokenCommand::Transfer {
        token_id,
        to,
        amount,
    })
    .unwrap_or_default();
    Command {
        prism: "token".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn balance_of(state: &StateTree, token_id: &Hash, who: &PartyId) -> u128 {
    TokenPrism::balance(state, token_id, who)
}

pub fn token_meta(state: &StateTree, token_id: &Hash) -> Option<TokenMeta> {
    state
        .get_json::<TokenMeta>(&TokenPrism::meta_key(token_id))
        .ok()
        .flatten()
}

pub fn credit(
    state: &mut StateTree,
    token_id: &Hash,
    who: &PartyId,
    amount: u128,
) -> Result<(), PrismError> {
    let bal = TokenPrism::balance(state, token_id, who);
    TokenPrism::set_balance(
        state,
        token_id,
        who,
        bal.checked_add(amount)
            .ok_or_else(|| PrismError::Internal("balance overflow".into()))?,
    )?;
    if let Some(mut meta) = token_meta(state, token_id) {
        meta.total_supply = meta.total_supply.saturating_add(amount);
        state
            .put_json(TokenPrism::meta_key(token_id), &meta)
            .map_err(|e| PrismError::Internal(e.to_string()))?;
    }
    Ok(())
}

pub fn debit(
    state: &mut StateTree,
    token_id: &Hash,
    who: &PartyId,
    amount: u128,
) -> Result<(), PrismError> {
    let bal = TokenPrism::balance(state, token_id, who);
    if bal < amount {
        return Err(PrismError::LimitExceeded("insufficient balance".into()));
    }
    TokenPrism::set_balance(state, token_id, who, bal - amount)
}

pub fn move_balance(
    state: &mut StateTree,
    token_id: &Hash,
    from: &PartyId,
    to: &PartyId,
    amount: u128,
) -> Result<(), PrismError> {
    debit(state, token_id, from, amount)?;
    let to_bal = TokenPrism::balance(state, token_id, to);
    TokenPrism::set_balance(
        state,
        token_id,
        to,
        to_bal
            .checked_add(amount)
            .ok_or_else(|| PrismError::Internal("balance overflow".into()))?,
    )
}

pub fn burn_from(
    state: &mut StateTree,
    token_id: &Hash,
    who: &PartyId,
    amount: u128,
) -> Result<(), PrismError> {
    debit(state, token_id, who, amount)?;
    if let Some(mut meta) = token_meta(state, token_id) {
        meta.total_supply = meta.total_supply.saturating_sub(amount);
        state
            .put_json(TokenPrism::meta_key(token_id), &meta)
            .map_err(|e| PrismError::Internal(e.to_string()))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeOutcome {
    pub charged: u128,
    pub burned: u128,
    pub rewarded: u128,
}

pub fn collect_fee(
    state: &mut StateTree,
    token_id: &Hash,
    payer: &PartyId,
    proposer: &PartyId,
    fee: u128,
    burn_bps: u16,
) -> Result<FeeOutcome, PrismError> {
    let bal = TokenPrism::balance(state, token_id, payer);
    let charged = fee.min(bal);
    if charged == 0 {
        return Ok(FeeOutcome::default());
    }
    let burned = charged * (burn_bps.min(10_000) as u128) / 10_000;
    let rewarded = charged - burned;
    TokenPrism::set_balance(state, token_id, payer, bal - charged)?;
    if rewarded > 0 {
        let pbal = TokenPrism::balance(state, token_id, proposer);
        TokenPrism::set_balance(
            state,
            token_id,
            proposer,
            pbal.checked_add(rewarded)
                .ok_or_else(|| PrismError::Internal("reward overflow".into()))?,
        )?;
    }
    if burned > 0 {
        if let Some(mut meta) = token_meta(state, token_id) {
            meta.total_supply = meta.total_supply.saturating_sub(burned);
            state
                .put_json(TokenPrism::meta_key(token_id), &meta)
                .map_err(|e| PrismError::Internal(e.to_string()))?;
        }
    }
    Ok(FeeOutcome {
        charged,
        burned,
        rewarded,
    })
}

pub fn seed_native_token(
    state: &mut StateTree,
    name: &str,
    symbol: &str,
    decimals: u8,
    treasury: &PartyId,
    allocations: &[(PartyId, u128)],
) -> Result<Hash, PrismError> {
    let token_id = native_token_id();
    if state.contains(&TokenPrism::meta_key(&token_id)) {
        return Ok(token_id);
    }
    let total: u128 = allocations.iter().map(|(_, a)| *a).sum();
    let meta = TokenMeta {
        token_id,
        name: name.to_string(),
        symbol: symbol.to_string(),
        decimals,
        total_supply: total,
        owner: treasury.clone(),
        mintable: true,
    };
    state
        .put_json(TokenPrism::meta_key(&token_id), &meta)
        .map_err(|e| PrismError::Internal(e.to_string()))?;
    for (who, amount) in allocations {
        let cur = TokenPrism::balance(state, &token_id, who);
        TokenPrism::set_balance(
            state,
            &token_id,
            who,
            cur.checked_add(*amount)
                .ok_or_else(|| PrismError::Internal("allocation overflow".into()))?,
        )?;
    }
    Ok(token_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_id_from(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<TokenEvent>(&out.events[0].payload).unwrap() {
            TokenEvent::Created { token_id, .. } => token_id,
            _ => panic!("expected Created"),
        }
    }

    #[test]
    fn create_and_transfer() {
        let p = TokenPrism::new();
        let mut s = StateTree::new();
        let create = create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            18,
            1000,
            true,
        );
        let out = p.handle(&create, &mut s).unwrap();
        let id = token_id_from(&out);
        assert_eq!(balance_of(&s, &id, &PartyId::new("alice")), 1000);

        let xfer = transfer_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            id,
            PartyId::new("bob"),
            250,
        );
        p.handle(&xfer, &mut s).unwrap();
        assert_eq!(balance_of(&s, &id, &PartyId::new("alice")), 750);
        assert_eq!(balance_of(&s, &id, &PartyId::new("bob")), 250);
    }

    #[test]
    fn cannot_overspend() {
        let p = TokenPrism::new();
        let mut s = StateTree::new();
        let create = create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            18,
            100,
            false,
        );
        let out = p.handle(&create, &mut s).unwrap();
        let id = token_id_from(&out);
        let xfer = transfer_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            id,
            PartyId::new("bob"),
            500,
        );
        assert!(p.handle(&xfer, &mut s).is_err());
    }

    #[test]
    fn approve_and_transfer_from() {
        let p = TokenPrism::new();
        let mut s = StateTree::new();
        let create = create_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "Gold",
            "GLD",
            18,
            1000,
            false,
        );
        let id = token_id_from(&p.handle(&create, &mut s).unwrap());

        let approve = Command {
            prism: "token".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&TokenCommand::Approve {
                token_id: id,
                spender: PartyId::new("carol"),
                amount: 300,
            })
            .unwrap(),
            nonce: 1,
        };
        p.handle(&approve, &mut s).unwrap();

        let tf = Command {
            prism: "token".into(),
            submitter: PartyId::new("carol"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&TokenCommand::TransferFrom {
                token_id: id,
                from: PartyId::new("alice"),
                to: PartyId::new("dave"),
                amount: 200,
            })
            .unwrap(),
            nonce: 0,
        };
        p.handle(&tf, &mut s).unwrap();
        assert_eq!(balance_of(&s, &id, &PartyId::new("dave")), 200);
        assert_eq!(balance_of(&s, &id, &PartyId::new("alice")), 800);
    }

    #[test]
    fn native_token_seed_and_balances() {
        let mut s = StateTree::new();
        let id = seed_native_token(
            &mut s,
            "Veilux",
            "LUX",
            18,
            &PartyId::new("treasury"),
            &[
                (PartyId::new("treasury"), 700),
                (PartyId::new("alice"), 300),
            ],
        )
        .unwrap();
        assert_eq!(id, native_token_id());
        assert_eq!(token_meta(&s, &id).unwrap().total_supply, 1000);
        assert_eq!(balance_of(&s, &id, &PartyId::new("alice")), 300);
        seed_native_token(&mut s, "X", "X", 0, &PartyId::new("t"), &[]).unwrap();
        assert_eq!(token_meta(&s, &id).unwrap().total_supply, 1000);
    }

    #[test]
    fn fee_splits_burn_and_reward_conserving_value() {
        let mut s = StateTree::new();
        let id = seed_native_token(
            &mut s,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("alice"), 10_000)],
        )
        .unwrap();
        let supply_before = token_meta(&s, &id).unwrap().total_supply;

        let out = collect_fee(
            &mut s,
            &id,
            &PartyId::new("alice"),
            &PartyId::new("v1"),
            1_000,
            3_000,
        )
        .unwrap();
        assert_eq!(out.charged, 1_000);
        assert_eq!(out.burned, 300);
        assert_eq!(out.rewarded, 700);
        assert_eq!(balance_of(&s, &id, &PartyId::new("alice")), 9_000);
        assert_eq!(balance_of(&s, &id, &PartyId::new("v1")), 700);
        assert_eq!(
            token_meta(&s, &id).unwrap().total_supply,
            supply_before - 300
        );
    }

    #[test]
    fn fee_is_capped_at_payer_balance() {
        let mut s = StateTree::new();
        let id = seed_native_token(
            &mut s,
            "Veilux",
            "LUX",
            0,
            &PartyId::new("treasury"),
            &[(PartyId::new("poor"), 50)],
        )
        .unwrap();
        let out = collect_fee(
            &mut s,
            &id,
            &PartyId::new("poor"),
            &PartyId::new("v1"),
            1_000,
            0,
        )
        .unwrap();
        assert_eq!(out.charged, 50);
        assert_eq!(balance_of(&s, &id, &PartyId::new("poor")), 0);
        assert_eq!(balance_of(&s, &id, &PartyId::new("v1")), 50);
    }
}
