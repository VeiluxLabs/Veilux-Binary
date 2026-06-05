use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;

use veilux_kernel::{Command, PartyId, SignedCommand};

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("malformed public key")]
    BadPublicKey,
    #[error("malformed signature")]
    BadSignature,
    #[error("signature verification failed")]
    VerificationFailed,
    #[error("submitter party does not match signing key")]
    PartyKeyMismatch,
}

pub struct PartyIdentity {
    party: PartyId,
    signing_key: SigningKey,
}

impl PartyIdentity {
    pub fn generate(label: &str) -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        PartyIdentity {
            party: PartyId::new(label),
            signing_key,
        }
    }

    pub fn from_seed(label: &str, seed: &[u8; 32]) -> Self {
        PartyIdentity {
            party: PartyId::new(label),
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    pub fn party(&self) -> &PartyId {
        &self.party
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn secret_seed(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn sign(&self, command: Command) -> SignedCommand {
        let sig: Signature = self.signing_key.sign(&command.signing_bytes());
        SignedCommand {
            command,
            public_key: self.public_key().to_vec(),
            signature: sig.to_bytes().to_vec(),
        }
    }

    pub fn sign_bytes(&self, message: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes().to_vec()
    }
}

pub fn verify_bytes(
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), IdentityError> {
    let pk_bytes: [u8; 32] = public_key
        .try_into()
        .map_err(|_| IdentityError::BadPublicKey)?;
    let verifying_key =
        VerifyingKey::from_bytes(&pk_bytes).map_err(|_| IdentityError::BadPublicKey)?;
    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| IdentityError::BadSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key
        .verify(message, &signature)
        .map_err(|_| IdentityError::VerificationFailed)
}

pub fn verify_signed(signed: &SignedCommand) -> Result<(), IdentityError> {
    let pk_bytes: [u8; 32] = signed
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| IdentityError::BadPublicKey)?;
    let verifying_key =
        VerifyingKey::from_bytes(&pk_bytes).map_err(|_| IdentityError::BadPublicKey)?;

    let sig_bytes: [u8; 64] = signed
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| IdentityError::BadSignature)?;
    let signature = Signature::from_bytes(&sig_bytes);

    verifying_key
        .verify(&signed.command.signing_bytes(), &signature)
        .map_err(|_| IdentityError::VerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::Visibility;

    fn cmd(party: &PartyId, nonce: u64) -> Command {
        Command {
            prism: "storage".into(),
            submitter: party.clone(),
            visibility: Visibility::Public,
            payload: b"data".to_vec(),
            nonce,
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = PartyIdentity::generate("alice");
        let signed = id.sign(cmd(id.party(), 0));
        assert!(verify_signed(&signed).is_ok());
    }

    #[test]
    fn tampered_payload_fails() {
        let id = PartyIdentity::generate("alice");
        let mut signed = id.sign(cmd(id.party(), 0));
        signed.command.payload = b"tampered".to_vec();
        assert!(verify_signed(&signed).is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let id = PartyIdentity::generate("alice");
        let mut signed = id.sign(cmd(id.party(), 0));
        signed.command.nonce = 99;
        assert!(verify_signed(&signed).is_err());
    }

    #[test]
    fn seed_roundtrip_is_stable() {
        let id = PartyIdentity::generate("alice");
        let seed = id.secret_seed();
        let id2 = PartyIdentity::from_seed("alice", &seed);
        assert_eq!(id.public_key(), id2.public_key());
    }
}
