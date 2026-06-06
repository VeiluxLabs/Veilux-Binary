use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};

use veilux_kernel::{Event, Hash, PartyId};

#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    #[error("decryption failed (wrong key or tampered ciphertext)")]
    Decryption,

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("no key available for party {0}")]
    NoKey(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedView {
    pub commitment: Hash,
    pub recipient: PartyId,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

impl EncryptedView {
    pub fn seal(event: &Event, recipient: &PartyId, key: &[u8; 32]) -> Result<Self, ViewError> {
        let commitment = event.commitment();
        let plaintext =
            serde_json::to_vec(event).map_err(|e| ViewError::Serialization(e.to_string()))?;

        let nonce_bytes = derive_nonce(&commitment, recipient);
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|e| ViewError::Encryption(e.to_string()))?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &plaintext,
                    aad: commitment.as_bytes(),
                },
            )
            .map_err(|e| ViewError::Encryption(e.to_string()))?;

        Ok(EncryptedView {
            commitment,
            recipient: recipient.clone(),
            nonce: nonce_bytes,
            ciphertext,
        })
    }

    pub fn open(&self, key: &[u8; 32]) -> Result<Event, ViewError> {
        let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| ViewError::Decryption)?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&self.nonce),
                Payload {
                    msg: &self.ciphertext,
                    aad: self.commitment.as_bytes(),
                },
            )
            .map_err(|_| ViewError::Decryption)?;
        let event: Event = serde_json::from_slice(&plaintext)
            .map_err(|e| ViewError::Serialization(e.to_string()))?;
        Ok(event)
    }
}

fn derive_nonce(commitment: &Hash, recipient: &PartyId) -> [u8; 12] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"veilux/veil/nonce");
    hasher.update(commitment.as_bytes());
    hasher.update(recipient.0.as_bytes());
    let digest = hasher.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest.as_bytes()[..12]);
    nonce
}

#[derive(Clone)]
pub struct ViewKeyring {
    party: PartyId,
    seed: [u8; 32],
}

impl ViewKeyring {
    pub fn new(party: PartyId, seed: [u8; 32]) -> Self {
        Self { party, seed }
    }

    pub fn from_passphrase(party: PartyId, passphrase: &str) -> Self {
        let seed = *blake3::hash(passphrase.as_bytes()).as_bytes();
        Self { party, seed }
    }

    pub fn party(&self) -> &PartyId {
        &self.party
    }

    pub fn private_seed(&self) -> &[u8] {
        &self.seed
    }

    pub fn key_for(&self, view_id: &Hash) -> [u8; 32] {
        crate::derive_view_key(&self.seed, view_id)
    }

    pub fn x25519_secret(&self) -> x25519_dalek::StaticSecret {
        let mut h = blake3::Hasher::new();
        h.update(b"veilux/veil/x25519-static/v1");
        h.update(&self.seed);
        let bytes: [u8; 32] = *h.finalize().as_bytes();
        x25519_dalek::StaticSecret::from(bytes)
    }

    pub fn x25519_public(&self) -> [u8; 32] {
        x25519_dalek::PublicKey::from(&self.x25519_secret()).to_bytes()
    }

    pub fn seal(&self, event: &Event) -> Result<EncryptedView, ViewError> {
        let key = self.key_for(&event.commitment());
        EncryptedView::seal(event, &self.party, &key)
    }

    pub fn open(&self, view: &EncryptedView) -> Result<Event, ViewError> {
        if view.recipient != self.party {
            return Err(ViewError::NoKey(view.recipient.0.clone()));
        }
        let key = self.key_for(&view.commitment);
        view.open(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::Visibility;

    fn sample_event() -> Event {
        Event {
            source_command: Hash::digest(b"cmd"),
            prism: "token".into(),
            visibility: Visibility::Parties(vec![PartyId::new("alice"), PartyId::new("bob")]),
            payload: b"alice pays bob 5 LUX".to_vec(),
        }
    }

    #[test]
    fn seal_and_open_roundtrip() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-secret");
        let event = sample_event();
        let view = alice.seal(&event).unwrap();
        let opened = alice.open(&view).unwrap();
        assert_eq!(opened.payload, event.payload);
    }

    #[test]
    fn wrong_party_cannot_open() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-secret");
        let mallory = ViewKeyring::from_passphrase(PartyId::new("mallory"), "mallory-secret");
        let event = sample_event();
        let view = alice.seal(&event).unwrap();
        assert!(mallory.open(&view).is_err());
    }

    #[test]
    fn commitment_is_independent_of_recipient() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "a");
        let bob = ViewKeyring::from_passphrase(PartyId::new("bob"), "b");
        let event = sample_event();
        let va = EncryptedView::seal(
            &event,
            &PartyId::new("alice"),
            &alice.key_for(&event.commitment()),
        )
        .unwrap();
        let vb = EncryptedView::seal(
            &event,
            &PartyId::new("bob"),
            &bob.key_for(&event.commitment()),
        )
        .unwrap();
        assert_eq!(va.commitment, vb.commitment);
    }
}
