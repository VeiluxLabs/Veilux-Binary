use std::collections::HashMap;
use std::net::IpAddr;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use veilux_veil::{verify_bytes, PartyIdentity};
use x25519_dalek::{EphemeralSecret, PublicKey as XPublicKey};

const HANDSHAKE_DOMAIN: &[u8] = b"veilux/net-handshake/v1";
const SESSION_KDF_DOMAIN: &[u8] = b"veilux/net-session/v1";

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
    #[error("malformed ephemeral key")]
    BadEphemeralKey,
    #[error("frame decryption failed (tampered or out-of-order ciphertext)")]
    Decryption,
}

#[derive(Clone, Debug)]
pub struct PeerKey {
    pub party: String,
    pub public_key: Vec<u8>,
}

#[derive(Clone)]
pub struct AuthConfig {
    pub party: String,
    pub secret_seed: [u8; 32],
    pub peers: Vec<PeerKey>,
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

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub party: String,
    pub public_key: Vec<u8>,
}

pub struct Session {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_counter: u64,
    recv_counter: u64,
}

impl Session {
    fn new(send_key: [u8; 32], recv_key: [u8; 32]) -> Self {
        Session {
            send_cipher: ChaCha20Poly1305::new_from_slice(&send_key).expect("32-byte key"),
            recv_cipher: ChaCha20Poly1305::new_from_slice(&recv_key).expect("32-byte key"),
            send_counter: 0,
            recv_counter: 0,
        }
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, AuthError> {
        let nonce = counter_nonce(self.send_counter);
        self.send_counter = self.send_counter.wrapping_add(1);
        self.send_cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| AuthError::Decryption)
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, AuthError> {
        let nonce = counter_nonce(self.recv_counter);
        self.recv_counter = self.recv_counter.wrapping_add(1);
        self.recv_cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .map_err(|_| AuthError::Decryption)
    }

    pub fn split(self) -> (Sender, Receiver) {
        (
            Sender {
                cipher: self.send_cipher,
                counter: self.send_counter,
            },
            Receiver {
                cipher: self.recv_cipher,
                counter: self.recv_counter,
            },
        )
    }
}

pub struct Sender {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Sender {
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, AuthError> {
        let nonce = counter_nonce(self.counter);
        self.counter = self.counter.wrapping_add(1);
        self.cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| AuthError::Decryption)
    }
}

pub struct Receiver {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Receiver {
    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, AuthError> {
        let nonce = counter_nonce(self.counter);
        self.counter = self.counter.wrapping_add(1);
        self.cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .map_err(|_| AuthError::Decryption)
    }
}

fn counter_nonce(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

fn derive_session_keys(shared: &[u8; 32], initiator_eph: &[u8], responder_eph: &[u8]) -> [u8; 64] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SESSION_KDF_DOMAIN);
    hasher.update(shared);
    hasher.update(initiator_eph);
    hasher.update(responder_eph);
    let mut out = [0u8; 64];
    let mut reader = hasher.finalize_xof();
    reader.fill(&mut out);
    out
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "a", content = "d")]
enum AuthMsg {
    Hello {
        party: String,
        public_key: Vec<u8>,
        nonce: Vec<u8>,
        ephemeral: Vec<u8>,
    },
    Proof {
        signature: Vec<u8>,
    },
}

fn proof_message(challenger_nonce: &[u8], signer_party: &str, signer_ephemeral: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(HANDSHAKE_DOMAIN.len() + challenger_nonce.len() + 64);
    m.extend_from_slice(HANDSHAKE_DOMAIN);
    m.push(0xff);
    m.extend_from_slice(challenger_nonce);
    m.push(0xff);
    m.extend_from_slice(signer_party.as_bytes());
    m.push(0xff);
    m.extend_from_slice(signer_ephemeral);
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

pub async fn perform_handshake<R, W>(
    cfg: &AuthConfig,
    reader: &mut R,
    writer: &mut W,
) -> Result<(PeerInfo, Session), AuthError>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let identity = PartyIdentity::from_seed(&cfg.party, &cfg.secret_seed);
    let mut my_nonce = vec![0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut my_nonce);

    let my_eph_secret = EphemeralSecret::random_from_rng(OsRng);
    let my_eph_public = XPublicKey::from(&my_eph_secret);
    let my_eph_bytes = my_eph_public.as_bytes().to_vec();

    write_msg(
        writer,
        &AuthMsg::Hello {
            party: cfg.party.clone(),
            public_key: identity.public_key().to_vec(),
            nonce: my_nonce.clone(),
            ephemeral: my_eph_bytes.clone(),
        },
    )
    .await?;

    let (peer_party, peer_pubkey, peer_nonce, peer_eph) = match read_msg(reader).await? {
        AuthMsg::Hello {
            party,
            public_key,
            nonce,
            ephemeral,
        } => (party, public_key, nonce, ephemeral),
        _ => return Err(AuthError::Unexpected),
    };

    let table = cfg.peer_table();
    let expected = table
        .get(peer_party.as_str())
        .ok_or_else(|| AuthError::UnknownPeer(peer_party.clone()))?;
    if *expected != peer_pubkey.as_slice() {
        return Err(AuthError::KeyMismatch(peer_party));
    }

    let proof = identity.sign_bytes(&proof_message(&peer_nonce, &cfg.party, &my_eph_bytes));
    write_msg(writer, &AuthMsg::Proof { signature: proof }).await?;

    let peer_sig = match read_msg(reader).await? {
        AuthMsg::Proof { signature } => signature,
        _ => return Err(AuthError::Unexpected),
    };

    verify_bytes(
        &peer_pubkey,
        &proof_message(&my_nonce, &peer_party, &peer_eph),
        &peer_sig,
    )
    .map_err(|_| AuthError::BadProof)?;

    let peer_eph_arr: [u8; 32] = peer_eph
        .as_slice()
        .try_into()
        .map_err(|_| AuthError::BadEphemeralKey)?;
    let shared = my_eph_secret.diffie_hellman(&XPublicKey::from(peer_eph_arr));
    let session = build_session(shared.as_bytes(), &my_eph_bytes, &peer_eph);

    Ok((
        PeerInfo {
            party: peer_party,
            public_key: peer_pubkey,
        },
        session,
    ))
}

fn build_session(shared: &[u8; 32], my_eph: &[u8], peer_eph: &[u8]) -> Session {
    let (initiator, responder, i_am_initiator) = if my_eph < peer_eph {
        (my_eph, peer_eph, true)
    } else {
        (peer_eph, my_eph, false)
    };
    let keys = derive_session_keys(shared, initiator, responder);
    let mut key_a = [0u8; 32];
    let mut key_b = [0u8; 32];
    key_a.copy_from_slice(&keys[..32]);
    key_b.copy_from_slice(&keys[32..]);
    if i_am_initiator {
        Session::new(key_a, key_b)
    } else {
        Session::new(key_b, key_a)
    }
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
        assert_eq!(ra.0.party, "v2");
        assert_eq!(rb.0.party, "v1");

        let mut sa = ra.1;
        let mut sb = rb.1;
        let frame = sa.seal(b"proposal block 7").unwrap();
        assert_ne!(frame, b"proposal block 7", "frame must be ciphertext");
        let opened = sb.open(&frame).unwrap();
        assert_eq!(opened, b"proposal block 7");
    }

    #[tokio::test]
    async fn unknown_peer_is_rejected() {
        let (a, b) = tokio::io::duplex(4096);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);

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
