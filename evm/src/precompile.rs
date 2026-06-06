use crate::u256::U256;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

use crate::keccak256;

pub fn is_precompile(address: &U256) -> bool {
    matches!(low_address(address), 1..=7)
}

fn low_address(address: &U256) -> u64 {
    let b = address.to_big_endian();
    if b[..24].iter().any(|x| *x != 0) {
        return 0;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&b[24..]);
    u64::from_be_bytes(buf)
}

pub fn execute(address: &U256, input: &[u8]) -> Option<Vec<u8>> {
    match low_address(address) {
        1 => Some(ecrecover(input)),
        2 => Some(sha256(input)),
        3 => Some(ripemd160(input)),
        4 => Some(input.to_vec()),
        5 => Some(modexp(input)),
        6 => Some(crate::bn254::ec_add(input).unwrap_or_else(|| vec![0u8; 64])),
        7 => Some(crate::bn254::ec_mul(input).unwrap_or_else(|| vec![0u8; 64])),
        _ => None,
    }
}

pub fn gas_cost(address: &U256, input: &[u8]) -> u64 {
    let words = ((input.len() as u64) + 31) / 32;
    match low_address(address) {
        1 => 3_000,
        2 => 60 + 12 * words,
        3 => 600 + 120 * words,
        4 => 15 + 3 * words,
        5 => 200,
        6 => 150,
        7 => 6_000,
        _ => 0,
    }
}

fn ripemd160(input: &[u8]) -> Vec<u8> {
    let mut h = Ripemd160::new();
    h.update(input);
    let digest = h.finalize();
    let mut out = vec![0u8; 32];
    out[12..].copy_from_slice(&digest);
    out
}

fn ecrecover(input: &[u8]) -> Vec<u8> {
    let mut buf = [0u8; 128];
    let n = input.len().min(128);
    buf[..n].copy_from_slice(&input[..n]);

    let hash = &buf[0..32];
    let v = buf[63];
    let r = &buf[64..96];
    let s = &buf[96..128];

    if v != 27 && v != 28 {
        return vec![];
    }
    let rec = v - 27;

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return vec![];
    };
    let Some(recid) = RecoveryId::from_byte(rec) else {
        return vec![];
    };
    let Ok(vk) = VerifyingKey::recover_from_prehash(hash, &sig, recid) else {
        return vec![];
    };
    let uncompressed = vk.to_encoded_point(false);
    let body = &uncompressed.as_bytes()[1..];
    let digest = keccak256(body);
    let mut out = vec![0u8; 32];
    out[12..].copy_from_slice(&digest[12..]);
    out
}

fn sha256(input: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(input);
    h.finalize().to_vec()
}

fn modexp(input: &[u8]) -> Vec<u8> {
    let mut buf = [0u8; 96];
    let n = input.len().min(96);
    buf[..n].copy_from_slice(&input[..n]);
    let blen = be_usize(&buf[0..32]);
    let elen = be_usize(&buf[32..64]);
    let mlen = be_usize(&buf[64..96]);

    if blen > 32 || elen > 32 || mlen > 32 {
        return vec![0u8; mlen.min(32)];
    }

    let data = &input[96.min(input.len())..];
    let base = read_u256(data, 0, blen);
    let exp = read_u256(data, blen, elen);
    let modulus = read_u256(data, blen + elen, mlen);

    let result = if modulus.is_zero() {
        U256::ZERO
    } else {
        mod_exp(base, exp, modulus)
    };

    let full = result.to_big_endian();
    full[32 - mlen..].to_vec()
}

fn mod_exp(mut base: U256, mut exp: U256, modulus: U256) -> U256 {
    if modulus == U256::ONE {
        return U256::ZERO;
    }
    let mut result = U256::ONE;
    base = base.div_mod(modulus).1;
    while !exp.is_zero() {
        if exp.bit(0) {
            result = result.wrapping_mul(base).div_mod(modulus).1;
        }
        exp = exp.shr(1);
        base = base.wrapping_mul(base).div_mod(modulus).1;
    }
    result
}

fn be_usize(bytes: &[u8]) -> usize {
    let mut v = 0usize;
    for b in bytes {
        v = v.saturating_mul(256).saturating_add(*b as usize);
    }
    v
}

fn read_u256(data: &[u8], offset: usize, len: usize) -> U256 {
    if len == 0 {
        return U256::ZERO;
    }
    let mut buf = [0u8; 32];
    let start = offset.min(data.len());
    let end = (offset + len).min(data.len());
    if start < end {
        let slice = &data[start..end];
        buf[32 - slice.len()..].copy_from_slice(slice);
    }
    U256::from_big_endian(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        let out = sha256(b"abc");
        assert_eq!(
            hex::encode(out),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn ripemd160_known_vector() {
        let out = ripemd160(b"abc");
        assert_eq!(
            hex::encode(&out),
            "0000000000000000000000008eb208f7e05d987a9b044a8e98c6b087f15a0bfc",
            "ripemd160('abc') left-padded into a 32-byte word (Ethereum precompile layout)"
        );
    }

    #[test]
    fn identity_returns_input() {
        let addr = U256::from_u64(4);
        let data = vec![1u8, 2, 3, 4, 5];
        assert_eq!(execute(&addr, &data).unwrap(), data);
    }

    #[test]
    fn modexp_basic() {
        let mut input = Vec::new();
        input.extend_from_slice(&u256_be(1));
        input.extend_from_slice(&u256_be(1));
        input.extend_from_slice(&u256_be(1));
        input.push(0x03);
        input.push(0x02);
        input.push(0x05);
        let out = modexp(&input);
        assert_eq!(out, vec![0x04], "3^2 mod 5 = 4");
    }

    #[test]
    fn ecrecover_matches_keccak_address() {
        use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let vk = sk.verifying_key();
        let uncompressed = vk.to_encoded_point(false);
        let expected = keccak256(&uncompressed.as_bytes()[1..]);

        let msg_hash = keccak256(b"hello veilux");
        let (sig, recid): (Signature, RecoveryId) = sk.sign_prehash(&msg_hash).unwrap();
        let sig = sig.normalize_s().unwrap_or(sig);

        let mut input = Vec::new();
        input.extend_from_slice(&msg_hash);
        let mut v = [0u8; 32];
        v[31] = 27 + (recid.to_byte() & 1);
        input.extend_from_slice(&v);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&sig.s().to_bytes());

        let out = ecrecover(&input);
        assert_eq!(
            &out[12..],
            &expected[12..],
            "ecrecover must return the signer address"
        );
    }

    fn u256_be(v: u64) -> [u8; 32] {
        U256::from_u64(v).to_big_endian()
    }
}
