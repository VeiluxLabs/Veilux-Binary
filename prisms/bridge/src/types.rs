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

/// Foreign chains VEILUX can bridge to/from. The wire format for each is
/// abstracted: a foreign address is just opaque bytes (hex), interpreted by the
/// relayers for that chain.
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

/// A registered bridge connection to a foreign chain: its guardian (relayer)
/// set and the quorum needed to mint inbound transfers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub chain: ForeignChain,
    /// Guardian Ed25519 public keys (32 bytes each, hex-encoded).
    pub guardians: Vec<String>,
    /// Number of guardian signatures required to accept an inbound transfer.
    pub quorum: usize,
    /// Admin party allowed to update this config.
    pub admin: PartyId,
}

impl BridgeConfig {
    pub fn config_key(chain: ForeignChain) -> String {
        format!("bridge/config/{}", chain.as_u16())
    }
}

/// An outbound transfer: VEILUX -> foreign chain. Locked here, minted there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundTransfer {
    pub sequence: u64,
    pub chain: ForeignChain,
    pub sender: PartyId,
    /// Foreign recipient address (hex/opaque).
    pub recipient: String,
    /// Token id being bridged (VEILUX-side).
    pub token_id: Hash,
    #[serde(with = "u128_dec")]
    pub amount: u128,
}

impl OutboundTransfer {
    /// Canonical bytes guardians observe and sign on the foreign side.
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

/// A guardian-attested inbound transfer: foreign chain -> VEILUX.
/// Guardians sign the `digest()` off-chain; the bridge verifies quorum.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundTransfer {
    pub chain: ForeignChain,
    /// Monotonic per-chain sequence (anti-replay).
    pub sequence: u64,
    /// Foreign sender address (hex/opaque).
    pub foreign_sender: String,
    /// VEILUX recipient.
    pub recipient: PartyId,
    /// Wrapped token id to credit on VEILUX.
    pub token_id: Hash,
    #[serde(with = "u128_dec")]
    pub amount: u128,
}

impl InboundTransfer {
    /// The message guardians sign to attest this transfer happened abroad.
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

/// A single guardian signature over an inbound transfer digest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardianSignature {
    /// Guardian public key (32 bytes, hex).
    pub public_key: String,
    /// Ed25519 signature over the transfer digest (64 bytes, hex).
    pub signature: String,
}
