use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};

use veilux_kernel::{Command, Hash, PartyId};

use crate::view::{ViewError, ViewKeyring};

pub const PRIVATE_KEY_DOMAIN: &str = "veilux/veil/private-key/v1";
pub const PRIVATE_NONCE_DOMAIN: &str = "veilux/veil/private-nonce/v1";
pub const PRIVATE_COMMIT_DOMAIN: &str = "veilux/veil/private-commit/v1";

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

    fn expected_commitment(salt: &Hash, stakeholders: &[PartyId], shares: &[SealedShare]) -> Hash {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        parts.push(PRIVATE_COMMIT_DOMAIN.as_bytes().to_vec());
        parts.push(salt.as_bytes().to_vec());
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
        Self::expected_commitment(&self.salt, &self.stakeholders, &self.shares) == self.commitment
    }
}

fn private_key(party_seed: &[u8], salt: &Hash) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(PRIVATE_KEY_DOMAIN.as_bytes());
    h.update(party_seed);
    h.update(salt.as_bytes());
    *h.finalize().as_bytes()
}

fn private_nonce(salt: &Hash, party: &PartyId) -> [u8; 12] {
    let mut h = blake3::Hasher::new();
    h.update(PRIVATE_NONCE_DOMAIN.as_bytes());
    h.update(salt.as_bytes());
    h.update(party.0.as_bytes());
    let digest = h.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest.as_bytes()[..12]);
    nonce
}

pub fn seal_private(
    inner: &Command,
    stakeholders: &[PartyId],
    keyrings: &[ViewKeyring],
    salt: Hash,
) -> Result<PrivateEnvelope, ViewError> {
    let plaintext =
        serde_json::to_vec(inner).map_err(|e| ViewError::Serialization(e.to_string()))?;

    let mut shares = Vec::new();
    for party in stakeholders {
        let Some(keyring) = keyrings.iter().find(|k| k.party() == party) else {
            continue;
        };
        let key = private_key(keyring.private_seed(), &salt);
        let nonce = private_nonce(&salt, party);
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
            recipient: party.clone(),
            nonce,
            ciphertext,
        });
    }

    let commitment = PrivateEnvelope::expected_commitment(&salt, stakeholders, &shares);
    Ok(PrivateEnvelope {
        commitment,
        salt,
        stakeholders: stakeholders.to_vec(),
        shares,
    })
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

    let key = private_key(keyring.private_seed(), &envelope.salt);
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
    fn tampered_ciphertext_fails_to_open() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-seed");
        let stakeholders = vec![PartyId::new("alice")];
        let mut env = seal_private(
            &cmd("alice"),
            &stakeholders,
            &[alice.clone()],
            salt_for(b"r2"),
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
            seal_private(&cmd("alice"), &stakeholders, &[alice], salt_for(b"r3")).unwrap();
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
        let env = seal_private(&cmd("alice"), &stakeholders, &[alice], salt_for(b"r4")).unwrap();
        let serialized = serde_json::to_vec(&env).unwrap();
        let haystack = serialized.windows(8).any(|w| w == b"transfer");
        assert!(
            !haystack,
            "the plaintext payload must never appear in the serialized envelope"
        );
    }
}
