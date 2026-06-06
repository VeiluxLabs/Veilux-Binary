pub mod precompile;
pub mod rlp;
pub mod u256;
pub mod vm;

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

pub use rlp::{Rlp, RlpError};

#[derive(Debug, thiserror::Error)]
pub enum EvmError {
    #[error("rlp error: {0}")]
    Rlp(#[from] RlpError),
    #[error("transaction is not a legacy/EIP-155 transaction list of 9 items")]
    BadShape,
    #[error("unsupported transaction type byte {0:#04x} (only legacy/EIP-155 supported)")]
    UnsupportedType(u8),
    #[error("invalid signature recovery id")]
    BadRecoveryId,
    #[error("signature recovery failed")]
    RecoveryFailed,
    #[error("value too large")]
    Overflow,
}

pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

pub fn address_from_pubkey(uncompressed: &[u8]) -> [u8; 20] {
    let body = if uncompressed.len() == 65 {
        &uncompressed[1..]
    } else {
        uncompressed
    };
    let hash = keccak256(body);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

pub fn to_checksum_address(addr: &[u8; 20]) -> String {
    let hex_addr = hex::encode(addr);
    let hash = keccak256(hex_addr.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in hex_addr.chars().enumerate() {
        if c.is_ascii_digit() {
            out.push(c);
        } else {
            let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0xf;
            if nibble >= 8 {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct EthTx {
    pub nonce: u64,
    pub gas_price: u128,
    pub gas_limit: u64,
    pub to: Option<[u8; 20]>,
    pub value: u128,
    pub data: Vec<u8>,
    pub chain_id: Option<u64>,
    pub from: [u8; 20],
    pub hash: [u8; 32],
}

fn be_to_u128(bytes: &[u8]) -> Result<u128, EvmError> {
    if bytes.len() > 16 {
        return Err(EvmError::Overflow);
    }
    let mut v = 0u128;
    for b in bytes {
        v = (v << 8) | (*b as u128);
    }
    Ok(v)
}

pub fn decode_legacy_tx(raw: &[u8]) -> Result<EthTx, EvmError> {
    if let Some(first) = raw.first() {
        if *first < 0x80 {
            return Err(EvmError::UnsupportedType(*first));
        }
    }
    let decoded = rlp::decode(raw)?;
    let items = decoded.as_list()?;
    if items.len() != 9 {
        return Err(EvmError::BadShape);
    }

    let nonce = items[0].as_u64()?;
    let gas_price = be_to_u128(items[1].as_bytes()?)?;
    let gas_limit = items[2].as_u64()?;
    let to_bytes = items[3].as_bytes()?;
    let to = if to_bytes.is_empty() {
        None
    } else if to_bytes.len() == 20 {
        let mut a = [0u8; 20];
        a.copy_from_slice(to_bytes);
        Some(a)
    } else {
        return Err(EvmError::BadShape);
    };
    let value = be_to_u128(items[4].as_bytes()?)?;
    let data = items[5].as_bytes()?.to_vec();
    let v = items[6].as_u64()?;
    let r = items[7].as_bytes()?.to_vec();
    let s = items[8].as_bytes()?.to_vec();

    let (chain_id, recovery_id) = split_v(v)?;

    let signing_payload = if let Some(cid) = chain_id {
        encode_signing_list_eip155(nonce, gas_price, gas_limit, &to, value, &data, cid)
    } else {
        encode_signing_list_legacy(nonce, gas_price, gas_limit, &to, value, &data)
    };
    let sighash = keccak256(&signing_payload);

    let from = recover_address(&sighash, &r, &s, recovery_id)?;
    let hash = keccak256(raw);

    Ok(EthTx {
        nonce,
        gas_price,
        gas_limit,
        to,
        value,
        data,
        chain_id,
        from,
        hash,
    })
}

fn split_v(v: u64) -> Result<(Option<u64>, u8), EvmError> {
    if v == 27 || v == 28 {
        Ok((None, (v - 27) as u8))
    } else if v >= 35 {
        let chain_id = (v - 35) / 2;
        let rec = ((v - 35) % 2) as u8;
        Ok((Some(chain_id), rec))
    } else if v == 0 || v == 1 {
        Ok((None, v as u8))
    } else {
        Err(EvmError::BadRecoveryId)
    }
}

fn left_pad_32(bytes: &[u8]) -> Result<[u8; 32], EvmError> {
    if bytes.len() > 32 {
        return Err(EvmError::Overflow);
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(out)
}

fn recover_address(sighash: &[u8; 32], r: &[u8], s: &[u8], rec: u8) -> Result<[u8; 20], EvmError> {
    let r32 = left_pad_32(r)?;
    let s32 = left_pad_32(s)?;
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(&r32);
    sig_bytes[32..].copy_from_slice(&s32);
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| EvmError::RecoveryFailed)?;
    let recid = RecoveryId::from_byte(rec).ok_or(EvmError::BadRecoveryId)?;
    let vk = VerifyingKey::recover_from_prehash(sighash, &sig, recid)
        .map_err(|_| EvmError::RecoveryFailed)?;
    let uncompressed = vk.to_encoded_point(false);
    Ok(address_from_pubkey(uncompressed.as_bytes()))
}

fn u128_be_trimmed(v: u128) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    v.to_be_bytes()
        .iter()
        .copied()
        .skip_while(|&b| b == 0)
        .collect()
}

fn u64_be_trimmed(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    v.to_be_bytes()
        .iter()
        .copied()
        .skip_while(|&b| b == 0)
        .collect()
}

fn to_field(to: &Option<[u8; 20]>) -> Vec<u8> {
    match to {
        Some(a) => a.to_vec(),
        None => vec![],
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_signing_list_eip155(
    nonce: u64,
    gas_price: u128,
    gas_limit: u64,
    to: &Option<[u8; 20]>,
    value: u128,
    data: &[u8],
    chain_id: u64,
) -> Vec<u8> {
    let fields = [
        rlp::encode_str(&u64_be_trimmed(nonce)),
        rlp::encode_str(&u128_be_trimmed(gas_price)),
        rlp::encode_str(&u64_be_trimmed(gas_limit)),
        rlp::encode_str(&to_field(to)),
        rlp::encode_str(&u128_be_trimmed(value)),
        rlp::encode_str(data),
        rlp::encode_str(&u64_be_trimmed(chain_id)),
        rlp::encode_str(&[]),
        rlp::encode_str(&[]),
    ];
    rlp::encode_list(&fields)
}

fn encode_signing_list_legacy(
    nonce: u64,
    gas_price: u128,
    gas_limit: u64,
    to: &Option<[u8; 20]>,
    value: u128,
    data: &[u8],
) -> Vec<u8> {
    let fields = [
        rlp::encode_str(&u64_be_trimmed(nonce)),
        rlp::encode_str(&u128_be_trimmed(gas_price)),
        rlp::encode_str(&u64_be_trimmed(gas_limit)),
        rlp::encode_str(&to_field(to)),
        rlp::encode_str(&u128_be_trimmed(value)),
        rlp::encode_str(data),
    ];
    rlp::encode_list(&fields)
}

#[allow(clippy::too_many_arguments)]
pub fn sign_legacy_tx(
    secret_key: &[u8; 32],
    nonce: u64,
    gas_price: u128,
    gas_limit: u64,
    to: Option<[u8; 20]>,
    value: u128,
    data: &[u8],
    chain_id: u64,
) -> Result<Vec<u8>, EvmError> {
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::SigningKey;

    let signing = SigningKey::from_slice(secret_key).map_err(|_| EvmError::RecoveryFailed)?;
    let payload =
        encode_signing_list_eip155(nonce, gas_price, gas_limit, &to, value, data, chain_id);
    let sighash = keccak256(&payload);
    let (sig, recid): (Signature, RecoveryId) = signing
        .sign_prehash(&sighash)
        .map_err(|_| EvmError::RecoveryFailed)?;
    let sig = sig.normalize_s().unwrap_or(sig);
    let recid_byte = recid.to_byte() & 1;
    let v = chain_id * 2 + 35 + recid_byte as u64;
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();

    let fields = [
        rlp::encode_str(&u64_be_trimmed(nonce)),
        rlp::encode_str(&u128_be_trimmed(gas_price)),
        rlp::encode_str(&u64_be_trimmed(gas_limit)),
        rlp::encode_str(&to_field(&to)),
        rlp::encode_str(&u128_be_trimmed(value)),
        rlp::encode_str(data),
        rlp::encode_str(&u64_be_trimmed(v)),
        rlp::encode_str(trim_left(&r)),
        rlp::encode_str(trim_left(&s)),
    ];
    Ok(rlp::encode_list(&fields))
}

fn trim_left(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < b.len() && b[i] == 0 {
        i += 1;
    }
    &b[i..]
}

pub fn address_from_secret(secret_key: &[u8; 32]) -> Result<[u8; 20], EvmError> {
    use k256::ecdsa::SigningKey;
    let signing = SigningKey::from_slice(secret_key).map_err(|_| EvmError::RecoveryFailed)?;
    let vk = signing.verifying_key();
    let uncompressed = vk.to_encoded_point(false);
    Ok(address_from_pubkey(uncompressed.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_address_matches_eip55() {
        let addr = hex::decode("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap();
        let mut a = [0u8; 20];
        a.copy_from_slice(&addr);
        assert_eq!(
            to_checksum_address(&a),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );
    }

    #[test]
    fn keccak_empty() {
        assert_eq!(
            hex::encode(keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn decodes_and_recovers_eip155_mainnet_vector() {
        let raw = hex::decode("f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83").unwrap();
        let tx = decode_legacy_tx(&raw).unwrap();
        assert_eq!(tx.nonce, 9);
        assert_eq!(tx.gas_price, 20_000_000_000);
        assert_eq!(tx.gas_limit, 21_000);
        assert_eq!(tx.value, 1_000_000_000_000_000_000);
        assert_eq!(tx.chain_id, Some(1));
        assert_eq!(
            to_checksum_address(&tx.from),
            "0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F"
        );
        assert_eq!(
            "0x".to_string() + &hex::encode(tx.to.unwrap()),
            "0x3535353535353535353535353535353535353535"
        );
    }

    #[test]
    fn rejects_typed_transaction_envelope() {
        let raw = vec![0x02, 0xc0];
        assert!(matches!(
            decode_legacy_tx(&raw),
            Err(EvmError::UnsupportedType(0x02))
        ));
    }

    #[test]
    fn sign_then_decode_recovers_sender() {
        let sk = [7u8; 32];
        let expected = address_from_secret(&sk).unwrap();
        let to = [0x35u8; 20];
        let raw = sign_legacy_tx(&sk, 3, 1_000_000_000, 21_000, Some(to), 5_000, &[], 42).unwrap();
        let tx = decode_legacy_tx(&raw).unwrap();
        assert_eq!(tx.from, expected);
        assert_eq!(tx.chain_id, Some(42));
        assert_eq!(tx.nonce, 3);
        assert_eq!(tx.value, 5_000);
        assert_eq!(tx.to, Some(to));
    }
}
