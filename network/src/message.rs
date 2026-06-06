use serde::{Deserialize, Serialize};
use veilux_consensus::Vote;
use veilux_kernel::{Block, SignedCommand};
use veilux_veil::{PrivateEnvelope, RootAttestation};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", content = "d")]
pub enum NetMessage {
    Hello { node_id: String, height: u64 },
    Command(Box<SignedCommand>),
    Proposal { round: u32, block: Box<Block> },
    Vote(Box<Vote>),
    Block(Box<Block>),
    RequestBlocks { from_height: u64 },
    Blocks { blocks: Vec<Block> },
    ViewChange(Box<ViewChange>),
    Private(Box<PrivateEnvelope>),
    PrivateRoot(Box<RootAttestation>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewChange {
    pub height: u64,
    pub view: u32,
    pub voter: veilux_kernel::PartyId,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl ViewChange {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(48);
        v.extend_from_slice(b"veilux/view-change/v1");
        v.push(0xff);
        v.extend_from_slice(&self.height.to_le_bytes());
        v.extend_from_slice(&self.view.to_le_bytes());
        v.extend_from_slice(self.voter.0.as_bytes());
        v
    }
}

impl NetMessage {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn decode(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            NetMessage::Hello { .. } => "hello",
            NetMessage::Command(_) => "command",
            NetMessage::Proposal { .. } => "proposal",
            NetMessage::Vote(_) => "vote",
            NetMessage::Block(_) => "block",
            NetMessage::RequestBlocks { .. } => "request_blocks",
            NetMessage::Blocks { .. } => "blocks",
            NetMessage::ViewChange(_) => "view_change",
            NetMessage::Private(_) => "private",
            NetMessage::PrivateRoot(_) => "private_root",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::PartyId;

    #[test]
    fn hello_roundtrip() {
        let m = NetMessage::Hello {
            node_id: "n1".into(),
            height: 5,
        };
        let line = m.encode().unwrap();
        let back = NetMessage::decode(&line).unwrap();
        assert_eq!(back.kind(), "hello");
    }

    #[test]
    fn block_roundtrip() {
        let b = Block::genesis(PartyId::new("v1"), 1);
        let m = NetMessage::Block(Box::new(b));
        let line = m.encode().unwrap();
        let back = NetMessage::decode(&line).unwrap();
        assert_eq!(back.kind(), "block");
    }

    #[test]
    fn private_envelope_roundtrip() {
        use veilux_veil::{seal_private, ViewKeyring};
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "a");
        let inner = veilux_kernel::Command {
            prism: "token".into(),
            submitter: PartyId::new("alice"),
            visibility: veilux_kernel::Visibility::Parties(vec![PartyId::new("alice")]),
            payload: b"secret".to_vec(),
            nonce: 0,
        };
        let env = seal_private(
            &inner,
            &[PartyId::new("alice")],
            &[alice],
            veilux_kernel::Hash::digest(b"s"),
        )
        .unwrap();
        let m = NetMessage::Private(Box::new(env));
        let line = m.encode().unwrap();
        let back = NetMessage::decode(&line).unwrap();
        assert_eq!(back.kind(), "private");
        assert!(
            !line.contains("secret"),
            "plaintext must not appear on the wire"
        );
    }
}
