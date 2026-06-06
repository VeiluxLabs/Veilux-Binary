use std::collections::BTreeMap;

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

mod u128_map {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(v: &BTreeMap<String, u128>, s: S) -> Result<S::Ok, S::Error> {
        let as_str: BTreeMap<String, String> = v
            .iter()
            .map(|(k, val)| (k.clone(), val.to_string()))
            .collect();
        as_str.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<String, u128>, D::Error> {
        let as_str = BTreeMap::<String, String>::deserialize(d)?;
        as_str
            .into_iter()
            .map(|(k, v)| {
                v.parse::<u128>()
                    .map(|n| (k, n))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

const POOL_PREFIX: &str = "dex/pool/";
const FEE_BPS: u128 = 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pool {
    pub id: Hash,
    pub token_a: Hash,
    pub token_b: Hash,
    #[serde(with = "u128_dec")]
    pub reserve_a: u128,
    #[serde(with = "u128_dec")]
    pub reserve_b: u128,
    #[serde(with = "u128_dec")]
    pub total_shares: u128,
    #[serde(with = "u128_map")]
    pub shares: BTreeMap<String, u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DexCommand {
    CreatePool {
        token_a: Hash,
        token_b: Hash,
    },
    AddLiquidity {
        pool: Hash,
        #[serde(with = "u128_dec")]
        amount_a: u128,
        #[serde(with = "u128_dec")]
        amount_b: u128,
    },
    RemoveLiquidity {
        pool: Hash,
        #[serde(with = "u128_dec")]
        shares: u128,
    },
    Swap {
        pool: Hash,
        token_in: Hash,
        #[serde(with = "u128_dec")]
        amount_in: u128,
        #[serde(with = "u128_dec")]
        min_out: u128,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DexEvent {
    PoolCreated {
        pool: Hash,
        token_a: Hash,
        token_b: Hash,
    },
    LiquidityAdded {
        pool: Hash,
        provider: PartyId,
        #[serde(with = "u128_dec")]
        shares: u128,
    },
    LiquidityRemoved {
        pool: Hash,
        provider: PartyId,
        #[serde(with = "u128_dec")]
        amount_a: u128,
        #[serde(with = "u128_dec")]
        amount_b: u128,
    },
    Swapped {
        pool: Hash,
        trader: PartyId,
        token_in: Hash,
        #[serde(with = "u128_dec")]
        amount_in: u128,
        token_out: Hash,
        #[serde(with = "u128_dec")]
        amount_out: u128,
    },
}

fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[derive(Default)]
pub struct DexPrism;

impl DexPrism {
    pub fn new() -> Self {
        DexPrism
    }

    fn key(id: &Hash) -> String {
        format!("{POOL_PREFIX}{}", id.to_hex())
    }

    fn load(state: &StateTree, id: &Hash) -> Result<Pool, PrismError> {
        state
            .get_json::<Pool>(&Self::key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("pool {}", id.to_hex())))
    }

    fn save(state: &mut StateTree, pool: &Pool) -> Result<(), PrismError> {
        state
            .put_json(Self::key(&pool.id), pool)
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn event(cmd: &Command, payload: DexEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "dex".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }

    pub fn quote_out(reserve_in: u128, reserve_out: u128, amount_in: u128) -> u128 {
        if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
            return 0;
        }
        let amount_in_after_fee = amount_in.saturating_mul(10_000 - FEE_BPS);
        let numerator = amount_in_after_fee.saturating_mul(reserve_out);
        let denominator = reserve_in
            .saturating_mul(10_000)
            .saturating_add(amount_in_after_fee);
        numerator / denominator
    }
}

impl Prism for DexPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "dex",
            description: "Constant-product (x*y=k) AMM: pools, liquidity and swaps",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: DexCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;
        let caller = command.submitter.clone();

        match cmd {
            DexCommand::CreatePool { token_a, token_b } => {
                if token_a == token_b {
                    return Err(PrismError::InvalidPayload(
                        "pool needs two distinct tokens".into(),
                    ));
                }
                let (token_a, token_b) = if token_a.to_hex() <= token_b.to_hex() {
                    (token_a, token_b)
                } else {
                    (token_b, token_a)
                };
                let id = Hash::commit(
                    "dex/pool-id",
                    &[token_a.to_hex().as_bytes(), token_b.to_hex().as_bytes()],
                );
                if state.contains(&Self::key(&id)) {
                    return Err(PrismError::InvalidPayload("pool already exists".into()));
                }
                let pool = Pool {
                    id,
                    token_a,
                    token_b,
                    reserve_a: 0,
                    reserve_b: 0,
                    total_shares: 0,
                    shares: BTreeMap::new(),
                };
                Self::save(state, &pool)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        DexEvent::PoolCreated {
                            pool: id,
                            token_a,
                            token_b,
                        },
                    ),
                    4_000,
                ))
            }

            DexCommand::AddLiquidity {
                pool,
                amount_a,
                amount_b,
            } => {
                let mut p = Self::load(state, &pool)?;
                if amount_a == 0 || amount_b == 0 {
                    return Err(PrismError::InvalidPayload("amounts must be > 0".into()));
                }
                let minted = if p.total_shares == 0 {
                    isqrt(amount_a.saturating_mul(amount_b))
                } else {
                    let by_a = amount_a.saturating_mul(p.total_shares) / p.reserve_a;
                    let by_b = amount_b.saturating_mul(p.total_shares) / p.reserve_b;
                    by_a.min(by_b)
                };
                if minted == 0 {
                    return Err(PrismError::InvalidPayload(
                        "liquidity too small to mint shares".into(),
                    ));
                }
                debit(state, &p.token_a, &caller, amount_a)?;
                debit(state, &p.token_b, &caller, amount_b)?;
                p.reserve_a = p.reserve_a.saturating_add(amount_a);
                p.reserve_b = p.reserve_b.saturating_add(amount_b);
                p.total_shares = p.total_shares.saturating_add(minted);
                let entry = p.shares.entry(caller.0.clone()).or_insert(0);
                *entry = entry.saturating_add(minted);
                Self::save(state, &p)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        DexEvent::LiquidityAdded {
                            pool,
                            provider: caller,
                            shares: minted,
                        },
                    ),
                    3_000,
                ))
            }

            DexCommand::RemoveLiquidity { pool, shares } => {
                let mut p = Self::load(state, &pool)?;
                let held = p.shares.get(&caller.0).copied().unwrap_or(0);
                if shares == 0 || shares > held {
                    return Err(PrismError::LimitExceeded("not enough shares".into()));
                }
                let amount_a = p.reserve_a.saturating_mul(shares) / p.total_shares;
                let amount_b = p.reserve_b.saturating_mul(shares) / p.total_shares;
                p.reserve_a = p.reserve_a.saturating_sub(amount_a);
                p.reserve_b = p.reserve_b.saturating_sub(amount_b);
                p.total_shares = p.total_shares.saturating_sub(shares);
                let remaining = held - shares;
                if remaining == 0 {
                    p.shares.remove(&caller.0);
                } else {
                    p.shares.insert(caller.0.clone(), remaining);
                }
                credit(state, &p.token_a, &caller, amount_a)?;
                credit(state, &p.token_b, &caller, amount_b)?;
                Self::save(state, &p)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        DexEvent::LiquidityRemoved {
                            pool,
                            provider: caller,
                            amount_a,
                            amount_b,
                        },
                    ),
                    2_500,
                ))
            }

            DexCommand::Swap {
                pool,
                token_in,
                amount_in,
                min_out,
            } => {
                let mut p = Self::load(state, &pool)?;
                if amount_in == 0 {
                    return Err(PrismError::InvalidPayload("amount_in must be > 0".into()));
                }
                let (reserve_in, reserve_out, token_out) = if token_in == p.token_a {
                    (p.reserve_a, p.reserve_b, p.token_b)
                } else if token_in == p.token_b {
                    (p.reserve_b, p.reserve_a, p.token_a)
                } else {
                    return Err(PrismError::InvalidPayload("token not in pool".into()));
                };
                let amount_out = Self::quote_out(reserve_in, reserve_out, amount_in);
                if amount_out == 0 || amount_out < min_out {
                    return Err(PrismError::LimitExceeded(
                        "slippage: output below min_out".into(),
                    ));
                }
                debit(state, &token_in, &caller, amount_in)?;
                credit(state, &token_out, &caller, amount_out)?;
                if token_in == p.token_a {
                    p.reserve_a = p.reserve_a.saturating_add(amount_in);
                    p.reserve_b = p.reserve_b.saturating_sub(amount_out);
                } else {
                    p.reserve_b = p.reserve_b.saturating_add(amount_in);
                    p.reserve_a = p.reserve_a.saturating_sub(amount_out);
                }
                Self::save(state, &p)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        DexEvent::Swapped {
                            pool,
                            trader: caller,
                            token_in,
                            amount_in,
                            token_out,
                            amount_out,
                        },
                    ),
                    2_000,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<DexCommand>(&command.payload) {
            Ok(DexCommand::CreatePool { .. }) => 4_000,
            Ok(DexCommand::AddLiquidity { .. }) => 3_000,
            Ok(DexCommand::RemoveLiquidity { .. }) => 2_500,
            Ok(DexCommand::Swap { .. }) => 2_000,
            Err(_) => 1_000,
        }
    }
}

pub fn pool_of(state: &StateTree, id: &Hash) -> Option<Pool> {
    state.get_json(&DexPrism::key(id)).ok().flatten()
}

pub fn create_pool_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    token_a: Hash,
    token_b: Hash,
) -> Command {
    let payload =
        serde_json::to_vec(&DexCommand::CreatePool { token_a, token_b }).unwrap_or_default();
    Command {
        prism: "dex".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn add_liquidity_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    pool: Hash,
    amount_a: u128,
    amount_b: u128,
) -> Command {
    let payload = serde_json::to_vec(&DexCommand::AddLiquidity {
        pool,
        amount_a,
        amount_b,
    })
    .unwrap_or_default();
    Command {
        prism: "dex".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn swap_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    pool: Hash,
    token_in: Hash,
    amount_in: u128,
    min_out: u128,
) -> Command {
    let payload = serde_json::to_vec(&DexCommand::Swap {
        pool,
        token_in,
        amount_in,
        min_out,
    })
    .unwrap_or_default();
    Command {
        prism: "dex".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_token::{balance_of, seed_native_token, TokenPrism};

    fn make_token(s: &mut StateTree, who: &str, symbol: &str, amount: u128) -> Hash {
        let p = TokenPrism::new();
        let create = prism_token::create_command(
            PartyId::new(who),
            Visibility::Public,
            0,
            symbol,
            symbol,
            0,
            amount,
            false,
        );
        let out = p.handle(&create, s).unwrap();
        match serde_json::from_slice::<prism_token::TokenEvent>(&out.events[0].payload).unwrap() {
            prism_token::TokenEvent::Created { token_id, .. } => token_id,
            _ => panic!("expected Created"),
        }
    }

    fn pool_id(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<DexEvent>(&out.events[0].payload).unwrap() {
            DexEvent::PoolCreated { pool, .. } => pool,
            _ => panic!("expected PoolCreated"),
        }
    }

    fn setup() -> (StateTree, Hash, Hash, Hash, DexPrism) {
        let mut s = StateTree::new();
        let _ = seed_native_token(&mut s, "Veilux", "LUX", 0, &PartyId::new("treasury"), &[]);
        let a = make_token(&mut s, "lp", "AAA", 1_000_000);
        let b = make_token(&mut s, "lp", "BBB", 1_000_000);
        let p = DexPrism::new();
        let id = pool_id(
            &p.handle(
                &create_pool_command(PartyId::new("lp"), Visibility::Public, 1, a, b),
                &mut s,
            )
            .unwrap(),
        );
        (s, a, b, id, p)
    }

    #[test]
    fn swap_follows_constant_product_and_charges_fee() {
        let (mut s, a, b, id, p) = setup();
        p.handle(
            &add_liquidity_command(
                PartyId::new("lp"),
                Visibility::Public,
                2,
                id,
                100_000,
                100_000,
            ),
            &mut s,
        )
        .unwrap();

        let _ = make_token(&mut s, "trader", "AAA2", 0);
        prism_token::credit(&mut s, &a, &PartyId::new("trader"), 10_000).unwrap();

        let before = balance_of(&s, &b, &PartyId::new("trader"));
        p.handle(
            &swap_command(
                PartyId::new("trader"),
                Visibility::Public,
                0,
                id,
                a,
                10_000,
                1,
            ),
            &mut s,
        )
        .unwrap();
        let got = balance_of(&s, &b, &PartyId::new("trader")) - before;

        let expected = DexPrism::quote_out(100_000, 100_000, 10_000);
        assert_eq!(got, expected);
        assert!(got > 0 && got < 10_000, "fee + slippage means out < in");

        let pool = pool_of(&s, &id).unwrap();
        assert_eq!(pool.reserve_a, 110_000);
        assert_eq!(pool.reserve_b, 100_000 - got);
    }

    #[test]
    fn add_then_remove_liquidity_round_trips() {
        let (mut s, _a, _b, id, p) = setup();
        let out = p
            .handle(
                &add_liquidity_command(
                    PartyId::new("lp"),
                    Visibility::Public,
                    2,
                    id,
                    40_000,
                    90_000,
                ),
                &mut s,
            )
            .unwrap();
        let shares = match serde_json::from_slice::<DexEvent>(&out.events[0].payload).unwrap() {
            DexEvent::LiquidityAdded { shares, .. } => shares,
            _ => panic!(),
        };
        let rm = Command {
            prism: "dex".into(),
            submitter: PartyId::new("lp"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&DexCommand::RemoveLiquidity { pool: id, shares }).unwrap(),
            nonce: 3,
        };
        p.handle(&rm, &mut s).unwrap();
        let pool = pool_of(&s, &id).unwrap();
        assert_eq!(pool.total_shares, 0);
        assert_eq!(pool.reserve_a, 0);
        assert_eq!(pool.reserve_b, 0);
    }

    #[test]
    fn swap_respects_min_out_slippage_guard() {
        let (mut s, a, _b, id, p) = setup();
        p.handle(
            &add_liquidity_command(
                PartyId::new("lp"),
                Visibility::Public,
                2,
                id,
                100_000,
                100_000,
            ),
            &mut s,
        )
        .unwrap();
        prism_token::credit(&mut s, &a, &PartyId::new("trader"), 10_000).unwrap();
        let swap = swap_command(
            PartyId::new("trader"),
            Visibility::Public,
            0,
            id,
            a,
            10_000,
            9_999_999,
        );
        assert!(
            p.handle(&swap, &mut s).is_err(),
            "min_out too high must revert"
        );
    }

    #[test]
    fn cannot_create_pool_with_same_token() {
        let (mut s, a, _b, _id, p) = setup();
        let bad = create_pool_command(PartyId::new("lp"), Visibility::Public, 9, a, a);
        assert!(p.handle(&bad, &mut s).is_err());
    }
}
