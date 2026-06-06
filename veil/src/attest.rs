use serde::{Deserialize, Serialize};

use veilux_kernel::{Hash, PartyId};

use crate::identity::{verify_bytes, IdentityError, PartyIdentity};

pub const ATTEST_DOMAIN: &[u8] = b"veilux/veil/private-root-attestation/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootAttestation {
    pub party: PartyId,
    pub commitment: Hash,
    pub private_root: Hash,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

pub fn attestation_message(commitment: &Hash, private_root: &Hash, party: &PartyId) -> Vec<u8> {
    let mut m = Vec::with_capacity(ATTEST_DOMAIN.len() + 96);
    m.extend_from_slice(ATTEST_DOMAIN);
    m.push(0xff);
    m.extend_from_slice(commitment.as_bytes());
    m.push(0xff);
    m.extend_from_slice(private_root.as_bytes());
    m.push(0xff);
    m.extend_from_slice(party.0.as_bytes());
    m
}

impl RootAttestation {
    pub fn create(identity: &PartyIdentity, commitment: Hash, private_root: Hash) -> Self {
        let party = identity.party().clone();
        let msg = attestation_message(&commitment, &private_root, &party);
        RootAttestation {
            party,
            commitment,
            private_root,
            public_key: identity.public_key().to_vec(),
            signature: identity.sign_bytes(&msg),
        }
    }

    pub fn verify(&self) -> Result<(), IdentityError> {
        let msg = attestation_message(&self.commitment, &self.private_root, &self.party);
        verify_bytes(&self.public_key, &msg, &self.signature)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AttestationBook {
    pub entries: Vec<RootAttestation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestationOutcome {
    Recorded,
    Duplicate,
    Divergence { existing: Hash, incoming: Hash },
    Invalid,
}

impl AttestationBook {
    pub fn record(&mut self, attestation: RootAttestation) -> AttestationOutcome {
        if attestation.verify().is_err() {
            return AttestationOutcome::Invalid;
        }
        for e in &self.entries {
            if e.commitment == attestation.commitment && e.party == attestation.party {
                if e.private_root == attestation.private_root {
                    return AttestationOutcome::Duplicate;
                }
                return AttestationOutcome::Divergence {
                    existing: e.private_root,
                    incoming: attestation.private_root,
                };
            }
        }

        if let Some(peer) = self
            .entries
            .iter()
            .find(|e| e.commitment == attestation.commitment && e.party != attestation.party)
        {
            if peer.private_root != attestation.private_root {
                let existing = peer.private_root;
                let incoming = attestation.private_root;
                self.entries.push(attestation);
                return AttestationOutcome::Divergence { existing, incoming };
            }
        }

        self.entries.push(attestation);
        AttestationOutcome::Recorded
    }

    pub fn agreement(&self, commitment: &Hash) -> Vec<&RootAttestation> {
        self.entries
            .iter()
            .filter(|e| &e.commitment == commitment)
            .collect()
    }

    pub fn is_consistent(&self, commitment: &Hash) -> bool {
        let mut root: Option<Hash> = None;
        for e in self.entries.iter().filter(|e| &e.commitment == commitment) {
            match root {
                None => root = Some(e.private_root),
                Some(r) if r != e.private_root => return false,
                _ => {}
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str) -> PartyIdentity {
        let mut seed = [0u8; 32];
        let b = name.as_bytes();
        seed[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
        PartyIdentity::from_seed(name, &seed)
    }

    #[test]
    fn valid_attestation_verifies_and_records() {
        let alice = identity("alice");
        let att = RootAttestation::create(&alice, Hash::digest(b"c1"), Hash::digest(b"root1"));
        assert!(att.verify().is_ok());
        let mut book = AttestationBook::default();
        assert_eq!(book.record(att), AttestationOutcome::Recorded);
    }

    #[test]
    fn tampered_attestation_is_invalid() {
        let alice = identity("alice");
        let mut att = RootAttestation::create(&alice, Hash::digest(b"c1"), Hash::digest(b"root1"));
        att.private_root = Hash::digest(b"forged");
        assert!(att.verify().is_err());
        let mut book = AttestationBook::default();
        assert_eq!(book.record(att), AttestationOutcome::Invalid);
    }

    #[test]
    fn agreeing_stakeholders_are_consistent() {
        let alice = identity("alice");
        let bob = identity("bob");
        let commitment = Hash::digest(b"shared-tx");
        let root = Hash::digest(b"same-root");
        let mut book = AttestationBook::default();
        assert_eq!(
            book.record(RootAttestation::create(&alice, commitment, root)),
            AttestationOutcome::Recorded
        );
        assert_eq!(
            book.record(RootAttestation::create(&bob, commitment, root)),
            AttestationOutcome::Recorded
        );
        assert!(book.is_consistent(&commitment));
        assert_eq!(book.agreement(&commitment).len(), 2);
    }

    #[test]
    fn divergent_stakeholders_are_detected() {
        let alice = identity("alice");
        let bob = identity("bob");
        let commitment = Hash::digest(b"shared-tx");
        let mut book = AttestationBook::default();
        book.record(RootAttestation::create(
            &alice,
            commitment,
            Hash::digest(b"root-A"),
        ));
        let outcome = book.record(RootAttestation::create(
            &bob,
            commitment,
            Hash::digest(b"root-B"),
        ));
        assert!(matches!(outcome, AttestationOutcome::Divergence { .. }));
        assert!(
            !book.is_consistent(&commitment),
            "two stakeholders reporting different private roots must be flagged inconsistent"
        );
    }
}
