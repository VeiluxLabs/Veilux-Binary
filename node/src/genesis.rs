use serde::{Deserialize, Serialize};

use prism_token::seed_native_token;
use veilux_kernel::{PartyId, StateTree};

/// Genesis configuration for a VEILUX chain. The native token's name, symbol,
/// decimals, and the initial supply distribution are all chosen here, so a new
/// network can pick its own token identity and total supply.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainSpec {
    pub token_name: String,
    pub token_symbol: String,
    pub token_decimals: u8,
    /// Treasury/admin party that owns the native token metadata (and may mint
    /// if mintable). Receives any unallocated remainder implicitly via its own
    /// allocation entry.
    pub treasury: String,
    /// Initial balances: (party, whole-token amount). The total supply is the
    /// sum of these, scaled by `token_decimals`.
    pub allocations: Vec<GenesisAlloc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAlloc {
    pub party: String,
    /// Amount in whole tokens (will be multiplied by 10^token_decimals).
    pub amount: u64,
}

impl Default for ChainSpec {
    fn default() -> Self {
        ChainSpec {
            token_name: "Veilux".to_string(),
            token_symbol: "LUX".to_string(),
            token_decimals: 18,
            treasury: "treasury".to_string(),
            allocations: vec![GenesisAlloc {
                party: "treasury".to_string(),
                amount: 1_000_000_000,
            }],
        }
    }
}

impl ChainSpec {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn total_supply_whole(&self) -> u128 {
        self.allocations.iter().map(|a| a.amount as u128).sum()
    }

    /// Seed the native token into a fresh chain's state. Deterministic and
    /// idempotent: every validator that shares this spec produces byte-identical
    /// state, and seeding an already-seeded chain is a no-op.
    pub fn seed(&self, state: &mut StateTree) -> anyhow::Result<()> {
        let scale = 10u128.pow(self.token_decimals as u32);
        let allocations: Vec<(PartyId, u128)> = self
            .allocations
            .iter()
            .map(|a| (PartyId::new(&a.party), a.amount as u128 * scale))
            .collect();
        seed_native_token(
            state,
            &self.token_name,
            &self.token_symbol,
            self.token_decimals,
            &PartyId::new(&self.treasury),
            &allocations,
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }
}
