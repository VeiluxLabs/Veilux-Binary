use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey as XPublicKey, StaticSecret};

use veilux_kernel::{Command, Hash, PartyId};

use crate::view::{ViewError, ViewKeyring};

pub const PRIVATE_KDF_DOMAIN: &str = "veilux/veil/private-x25519/v1";
pub const PRIVATE_NONCE_DOMAIN: &str = "veilux/veil/private-nonce/v1";
pub const PRIVATE_COMMIT_DOMAIN: &str = "veilux/veil/private-commit/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipient {
    pub party: PartyId,
    pub x25519_public: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SealedShare {
    pub recipient: PartyId,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivateEnvelope {
    pub commitment: Hash,
    pub salt: Hash,
    pub ephemeral_public: [u8; 32],
    pub stakeholders: Vec<PartyId>,
    pub shares: Vec<SealedShare>,
}

impl PrivateEnvelope {
    pub fn commitment(&self) -> Hash {
        self.commitment
    }

    pub fn is_stakeholder(&self, party: &PartyId) -> bool {
        self.stakeholders.iter().any(|p| p == party)
    }

    fn expected_commitment(
        salt: &Hash,
        ephemeral_public: &[u8; 32],
        stakeholders: &[PartyId],
        shares: &[SealedShare],
    ) -> Hash {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        parts.push(PRIVATE_COMMIT_DOMAIN.as_bytes().to_vec());
        parts.push(salt.as_bytes().to_vec());
        parts.push(ephemeral_public.to_vec());
        for p in stakeholders {
            parts.push(p.0.as_bytes().to_vec());
        }
        for s in shares {
            parts.push(s.recipient.0.as_bytes().to_vec());
            parts.push(s.nonce.to_vec());
            parts.push(s.ciphertext.clone());
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        Hash::commit("private-envelope", &refs)
    }

    pub fn verify_commitment(&self) -> bool {
        Self::expected_commitment(
            &self.salt,
            &self.ephemeral_public,
            &self.stakeholders,
            &self.shares,
        ) == self.commitment
    }
}

fn shared_key(shared: &[u8; 32], salt: &Hash, party: &PartyId) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(PRIVATE_KDF_DOMAIN.as_bytes());
    h.update(shared);
    h.update(salt.as_bytes());
    h.update(party.0.as_bytes());
    *h.finalize().as_bytes()
}

fn share_nonce(salt: &Hash, party: &PartyId) -> [u8; 12] {
    let mut h = blake3::Hasher::new();
    h.update(PRIVATE_NONCE_DOMAIN.as_bytes());
    h.update(salt.as_bytes());
    h.update(party.0.as_bytes());
    let digest = h.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest.as_bytes()[..12]);
    nonce
}

pub fn recipients_from_keyrings(parties: &[PartyId], keyrings: &[ViewKeyring]) -> Vec<Recipient> {
    parties
        .iter()
        .filter_map(|p| {
            keyrings.iter().find(|k| k.party() == p).map(|k| Recipient {
                party: p.clone(),
                x25519_public: k.x25519_public(),
            })
        })
        .collect()
}

pub fn seal_private_to(
    inner: &Command,
    recipients: &[Recipient],
    salt: Hash,
) -> Result<PrivateEnvelope, ViewError> {
    let plaintext =
        serde_json::to_vec(inner).map_err(|e| ViewError::Serialization(e.to_string()))?;

    let ephemeral = StaticSecret::random_from_rng(rand::rngs::OsRng);
    let ephemeral_public = XPublicKey::from(&ephemeral).to_bytes();

    let mut shares = Vec::new();
    let mut stakeholders = Vec::new();

    for r in recipients {
        let recipient_pub = XPublicKey::from(r.x25519_public);
        let shared = ephemeral.diffie_hellman(&recipient_pub);
        let key = shared_key(shared.as_bytes(), &salt, &r.party);
        let nonce = share_nonce(&salt, &r.party);
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| ViewError::Encryption(e.to_string()))?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: salt.as_bytes(),
                },
            )
            .map_err(|e| ViewError::Encryption(e.to_string()))?;
        shares.push(SealedShare {
            recipient: r.party.clone(),
            nonce,
            ciphertext,
        });
        stakeholders.push(r.party.clone());
    }

    let commitment =
        PrivateEnvelope::expected_commitment(&salt, &ephemeral_public, &stakeholders, &shares);
    Ok(PrivateEnvelope {
        commitment,
        salt,
        ephemeral_public,
        stakeholders,
        shares,
    })
}

pub fn seal_private(
    inner: &Command,
    stakeholders: &[PartyId],
    keyrings: &[ViewKeyring],
    salt: Hash,
) -> Result<PrivateEnvelope, ViewError> {
    let recipients = recipients_from_keyrings(stakeholders, keyrings);
    seal_private_to(inner, &recipients, salt)
}

pub fn open_private(
    envelope: &PrivateEnvelope,
    keyring: &ViewKeyring,
) -> Result<Command, ViewError> {
    let party = keyring.party();
    let share = envelope
        .shares
        .iter()
        .find(|s| s.recipient == *party)
        .ok_or_else(|| ViewError::NoKey(party.0.clone()))?;

    let secret = keyring.x25519_secret();
    let eph_pub = XPublicKey::from(envelope.ephemeral_public);
    let shared = secret.diffie_hellman(&eph_pub);
    let key = shared_key(shared.as_bytes(), &envelope.salt, party);

    let cipher = ChaCha20Poly1305::new_from_slice(&key).map_err(|_| ViewError::Decryption)?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&share.nonce),
            Payload {
                msg: &share.ciphertext,
                aad: envelope.salt.as_bytes(),
            },
        )
        .map_err(|_| ViewError::Decryption)?;
    let command: Command =
        serde_json::from_slice(&plaintext).map_err(|e| ViewError::Serialization(e.to_string()))?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::Visibility;

    fn cmd(submitter: &str) -> Command {
        Command {
            prism: "token".into(),
            submitter: PartyId::new(submitter),
            visibility: Visibility::Parties(vec![PartyId::new("alice"), PartyId::new("bob")]),
            payload: b"transfer 500 to bob, confidential".to_vec(),
            nonce: 3,
        }
    }

    fn salt_for(seed: &[u8]) -> Hash {
        Hash::digest(seed)
    }

    #[test]
    fn stakeholder_can_open_non_stakeholder_cannot() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let bob = ViewKeyring::from_passphrase(PartyId::new("bob"), "bob-seed");
        let mallory = ViewKeyring::from_passphrase(PartyId::new("mallory"), "mallory-seed");

        let stakeholders = vec![PartyId::new("alice"), PartyId::new("bob")];
        let env = seal_private(
            &cmd("alice"),
            &stakeholders,
            &[alice.clone(), bob.clone()],
            salt_for(b"r1"),
        )
        .unwrap();

        assert!(env.verify_commitment());

        let a = open_private(&env, &alice).unwrap();
        assert_eq!(a.payload, b"transfer 500 to bob, confidential");
        let b = open_private(&env, &bob).unwrap();
        assert_eq!(b.payload, a.payload);

        assert!(
            open_private(&env, &mallory).is_err(),
            "a non-stakeholder must not be able to open a private envelope"
        );
    }

    #[test]
    fn wrong_key_cannot_open_even_if_named_stakeholder() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let stakeholders = vec![PartyId::new("alice")];
        let env = seal_private(&cmd("alice"), &stakeholders, &[alice], salt_for(b"r2")).unwrap();

        let fake_alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "WRONG-seed");
        assert!(
            open_private(&env, &fake_alice).is_err(),
            "holding the right party name but the wrong X25519 secret must fail to decrypt"
        );
    }

    #[test]
    fn tampered_ciphertext_fails_to_open() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let stakeholders = vec![PartyId::new("alice")];
        let mut env = seal_private(
            &cmd("alice"),
            &stakeholders,
            &[alice.clone()],
            salt_for(b"r3"),
        )
        .unwrap();
        env.shares[0].ciphertext[0] ^= 0xff;
        assert!(open_private(&env, &alice).is_err());
    }

    #[test]
    fn commitment_detects_tampering() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let stakeholders = vec![PartyId::new("alice")];
        let mut env =
            seal_private(&cmd("alice"), &stakeholders, &[alice], salt_for(b"r4")).unwrap();
        assert!(env.verify_commitment());
        env.shares[0].ciphertext.push(0x00);
        assert!(
            !env.verify_commitment(),
            "mutating a share must break the public commitment"
        );
    }

    #[test]
    fn non_stakeholder_sees_only_commitment_and_ciphertext() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let stakeholders = vec![PartyId::new("alice")];
        let env = seal_private(&cmd("alice"), &stakeholders, &[alice], salt_for(b"r5")).unwrap();
        let serialized = serde_json::to_vec(&env).unwrap();
        let haystack = serialized.windows(8).any(|w| w == b"transfer");
        assert!(
            !haystack,
            "the plaintext payload must never appear in the serialized envelope"
        );
    }
}
