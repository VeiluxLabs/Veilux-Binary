use serde::{Deserialize, Serialize};
use veilux_kernel::{Hash, PartyId};

pub(crate) mod u128_dec {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForeignChain {
    Cosmos,
    Solana,
    Ethereum,
    Custom,
}

impl ForeignChain {
    pub fn as_u16(&self) -> u16 {
        match self {
            ForeignChain::Cosmos => 1,
            ForeignChain::Solana => 2,
            ForeignChain::Ethereum => 3,
            ForeignChain::Custom => 999,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub chain: ForeignChain,
    pub guardians: Vec<String>,
    pub quorum: usize,
    pub admin: PartyId,
}

impl BridgeConfig {
    pub fn config_key(chain: ForeignChain) -> String {
        format!("bridge/config/{}", chain.as_u16())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundTransfer {
    pub sequence: u64,
    pub chain: ForeignChain,
    pub sender: PartyId,
    pub recipient: String,
    pub token_id: Hash,
    #[serde(with = "u128_dec")]
    pub amount: u128,
}

impl OutboundTransfer {
    pub fn digest(&self) -> Hash {
        Hash::commit(
            "bridge/outbound",
            &[
                &self.sequence.to_le_bytes(),
                &self.chain.as_u16().to_le_bytes(),
                self.sender.0.as_bytes(),
                self.recipient.as_bytes(),
                self.token_id.as_bytes(),
                &self.amount.to_le_bytes(),
            ],
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundTransfer {
    pub chain: ForeignChain,
    pub sequence: u64,
    pub foreign_sender: String,
    pub recipient: PartyId,
    pub token_id: Hash,
    #[serde(with = "u128_dec")]
    pub amount: u128,
}

impl InboundTransfer {
    pub fn digest(&self) -> Hash {
        Hash::commit(
            "bridge/inbound",
            &[
                &self.chain.as_u16().to_le_bytes(),
                &self.sequence.to_le_bytes(),
                self.foreign_sender.as_bytes(),
                self.recipient.0.as_bytes(),
                self.token_id.as_bytes(),
                &self.amount.to_le_bytes(),
            ],
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardianSignature {
    pub public_key: String,
    pub signature: String,
}
