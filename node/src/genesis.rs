use serde::{Deserialize, Serialize};

use prism_token::seed_native_token;
use veilux_kernel::{PartyId, StateTree};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainSpec {
    pub token_name: String,
    pub token_symbol: String,
    pub token_decimals: u8,
    pub treasury: String,
    pub allocations: Vec<GenesisAlloc>,
    #[serde(default)]
    pub fee_price_per_gas: u128,
    #[serde(default = "default_burn_bps")]
    pub fee_burn_bps: u16,
}

fn default_burn_bps() -> u16 {
    5_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAlloc {
    pub party: String,
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
            fee_price_per_gas: 0,
            fee_burn_bps: default_burn_bps(),
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

    pub fn fee_policy(&self) -> crate::node::FeePolicy {
        crate::node::FeePolicy {
            price_per_gas: self.fee_price_per_gas,
            burn_bps: self.fee_burn_bps,
        }
    }

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
