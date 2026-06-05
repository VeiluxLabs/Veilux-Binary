use std::collections::HashMap;
use std::net::IpAddr;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use veilux_veil::{verify_bytes, PartyIdentity};

const HANDSHAKE_DOMAIN: &[u8] = b"veilux/net-handshake/v1";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode error: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("peer closed connection during handshake")]
    Closed,
    #[error("unexpected handshake message")]
    Unexpected,
    #[error("peer ip {0} is not on the allowlist")]
    IpNotAllowed(IpAddr),
    #[error("peer party {0} is not a known validator")]
    UnknownPeer(String),
    #[error("peer public key does not match the registered key for {0}")]
    KeyMismatch(String),
    #[error("peer proof signature is invalid")]
    BadProof,
}

/// A validator the local node is willing to talk to, identified by its party
/// name and the Ed25519 public key that name must sign with.
#[derive(Clone, Debug)]
pub struct PeerKey {
    pub party: String,
    pub public_key: Vec<u8>,
}

/// Transport authentication policy. When attached to a node, every peer
/// connection (inbound or outbound) must complete a mutual challenge-response
/// before any gossip is exchanged.
#[derive(Clone)]
pub struct AuthConfig {
    /// Local validator identity used to answer challenges.
    pub party: String,
    pub secret_seed: [u8; 32],
    /// Public keys this node will accept, keyed by party name.
    pub peers: Vec<PeerKey>,
    /// Source IPs allowed to dial this node. Empty = accept any source IP
    /// (key authentication is still enforced).
    pub ip_allowlist: Vec<IpAddr>,
}

impl AuthConfig {
    pub fn allows_ip(&self, ip: &IpAddr) -> bool {
        self.ip_allowlist.is_empty() || self.ip_allowlist.iter().any(|a| a == ip)
    }

    fn peer_table(&self) -> HashMap<&str, &[u8]> {
        self.peers
            .iter()
            .map(|p| (p.party.as_str(), p.public_key.as_slice()))
            .collect()
    }
}

/// Result of a successful handshake.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub party: String,
    pub public_key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "a", content = "d")]
enum AuthMsg {
    Hello {
        party: String,
        public_key: Vec<u8>,
        nonce: Vec<u8>,
    },
    Proof {
        signature: Vec<u8>,
    },
}

fn proof_message(challenger_nonce: &[u8], signer_party: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(HANDSHAKE_DOMAIN.len() + challenger_nonce.len() + 32);
    m.extend_from_slice(HANDSHAKE_DOMAIN);
    m.push(0xff);
    m.extend_from_slice(challenger_nonce);
    m.push(0xff);
    m.extend_from_slice(signer_party.as_bytes());
    m
}

async fn write_msg<W: AsyncWriteExt + Unpin>(w: &mut W, msg: &AuthMsg) -> Result<(), AuthError> {
    let line = serde_json::to_string(msg)?;
    w.write_all(line.as_bytes()).await?;
    w.write_all(b"\n").await?;
    Ok(())
}

async fn read_msg<R: AsyncBufReadExt + Unpin>(r: &mut R) -> Result<AuthMsg, AuthError> {
    let mut line = String::new();
    let n = r.read_line(&mut line).await?;
    if n == 0 {
        return Err(AuthError::Closed);
    }
    Ok(serde_json::from_str(line.trim())?)
}

/// Run the symmetric mutual handshake over an already-split connection.
///
/// Both endpoints send a `Hello` carrying their party, public key, and a fresh
/// random challenge, then each signs the *other* side's challenge and returns a
/// `Proof`. A connection only proceeds to gossip if both proofs verify against
/// keys on the local allowlist. This authenticates the peer (only the holder of
/// the registered secret key can answer) without trusting the network.
pub async fn perform_handshake<R, W>(
    cfg: &AuthConfig,
    reader: &mut R,
    writer: &mut W,
) -> Result<PeerInfo, AuthError>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let identity = PartyIdentity::from_seed(&cfg.party, &cfg.secret_seed);
    let mut my_nonce = vec![0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut my_nonce);

    write_msg(
        writer,
        &AuthMsg::Hello {
            party: cfg.party.clone(),
            public_key: identity.public_key().to_vec(),
            nonce: my_nonce.clone(),
        },
    )
    .await?;

    let (peer_party, peer_pubkey, peer_nonce) = match read_msg(reader).await? {
        AuthMsg::Hello {
            party,
            public_key,
            nonce,
        } => (party, public_key, nonce),
        _ => return Err(AuthError::Unexpected),
    };

    let table = cfg.peer_table();
    let expected = table
        .get(peer_party.as_str())
        .ok_or_else(|| AuthError::UnknownPeer(peer_party.clone()))?;
    if *expected != peer_pubkey.as_slice() {
        return Err(AuthError::KeyMismatch(peer_party));
    }

    let proof = identity.sign_bytes(&proof_message(&peer_nonce, &cfg.party));
    write_msg(writer, &AuthMsg::Proof { signature: proof }).await?;

    let peer_sig = match read_msg(reader).await? {
        AuthMsg::Proof { signature } => signature,
        _ => return Err(AuthError::Unexpected),
    };

    verify_bytes(
        &peer_pubkey,
        &proof_message(&my_nonce, &peer_party),
        &peer_sig,
    )
    .map_err(|_| AuthError::BadProof)?;

    Ok(PeerInfo {
        party: peer_party,
        public_key: peer_pubkey,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    fn identity_seed(party: &str) -> [u8; 32] {
        let mut s = [0u8; 32];
        let b = party.as_bytes();
        s[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
        s
    }

    fn pubkey_of(party: &str) -> Vec<u8> {
        PartyIdentity::from_seed(party, &identity_seed(party))
            .public_key()
            .to_vec()
    }

    fn cfg_for(party: &str, peers: &[&str]) -> AuthConfig {
        AuthConfig {
            party: party.to_string(),
            secret_seed: identity_seed(party),
            peers: peers
                .iter()
                .map(|p| PeerKey {
                    party: p.to_string(),
                    public_key: pubkey_of(p),
                })
                .collect(),
            ip_allowlist: vec![],
        }
    }

    #[tokio::test]
    async fn mutual_handshake_succeeds_between_known_validators() {
        let (a, b) = tokio::io::duplex(4096);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);

        let a_cfg = cfg_for("v1", &["v2"]);
        let b_cfg = cfg_for("v2", &["v1"]);

        let ja = tokio::spawn(async move {
            let mut r = BufReader::new(ar);
            let mut w = aw;
            perform_handshake(&a_cfg, &mut r, &mut w).await
        });
        let jb = tokio::spawn(async move {
            let mut r = BufReader::new(br);
            let mut w = bw;
            perform_handshake(&b_cfg, &mut r, &mut w).await
        });

        let ra = ja.await.unwrap().expect("v1 authenticates v2");
        let rb = jb.await.unwrap().expect("v2 authenticates v1");
        assert_eq!(ra.party, "v2");
        assert_eq!(rb.party, "v1");
    }

    #[tokio::test]
    async fn unknown_peer_is_rejected() {
        let (a, b) = tokio::io::duplex(4096);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);

        // v1 only trusts v2, but an unlisted "intruder" dials in.
        let a_cfg = cfg_for("v1", &["v2"]);
        let intruder = cfg_for("intruder", &["v1"]);

        let ja = tokio::spawn(async move {
            let mut r = BufReader::new(ar);
            let mut w = aw;
            perform_handshake(&a_cfg, &mut r, &mut w).await
        });
        let jb = tokio::spawn(async move {
            let mut r = BufReader::new(br);
            let mut w = bw;
            perform_handshake(&intruder, &mut r, &mut w).await
        });

        let result = ja.await.unwrap();
        assert!(matches!(result, Err(AuthError::UnknownPeer(_))));
        let _ = jb.await.unwrap();
    }

    #[tokio::test]
    async fn forged_key_for_known_party_is_rejected() {
        let (a, b) = tokio::io::duplex(4096);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);

        let a_cfg = cfg_for("v1", &["v2"]);
        // Attacker claims to be "v2" but signs with the wrong seed.
        let mut forger = cfg_for("v2", &["v1"]);
        forger.secret_seed = [9u8; 32];

        let ja = tokio::spawn(async move {
            let mut r = BufReader::new(ar);
            let mut w = aw;
            perform_handshake(&a_cfg, &mut r, &mut w).await
        });
        let jb = tokio::spawn(async move {
            let mut r = BufReader::new(br);
            let mut w = bw;
            perform_handshake(&forger, &mut r, &mut w).await
        });

        let result = ja.await.unwrap();
        assert!(matches!(result, Err(AuthError::KeyMismatch(_))));
        let _ = jb.await.unwrap();
    }

    #[test]
    fn ip_allowlist_enforced_when_set() {
        let mut cfg = cfg_for("v1", &["v2"]);
        assert!(cfg.allows_ip(&"203.0.113.7".parse().unwrap()));
        cfg.ip_allowlist = vec!["10.0.0.1".parse().unwrap()];
        assert!(cfg.allows_ip(&"10.0.0.1".parse().unwrap()));
        assert!(!cfg.allows_ip(&"203.0.113.7".parse().unwrap()));
    }
}
