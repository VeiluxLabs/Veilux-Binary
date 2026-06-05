#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct U256(pub [u64; 4]);

#[allow(clippy::should_implement_trait, clippy::needless_range_loop)]
impl U256 {
    pub const ZERO: U256 = U256([0, 0, 0, 0]);
    pub const ONE: U256 = U256([1, 0, 0, 0]);

    pub fn from_u64(v: u64) -> Self {
        U256([v, 0, 0, 0])
    }

    pub fn from_big_endian(bytes: &[u8]) -> Self {
        let mut buf = [0u8; 32];
        let n = bytes.len().min(32);
        buf[32 - n..].copy_from_slice(&bytes[bytes.len() - n..]);
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            let mut x = 0u64;
            for j in 0..8 {
                x = (x << 8) | buf[i * 8 + j] as u64;
            }
            limbs[3 - i] = x;
        }
        U256(limbs)
    }

    pub fn to_big_endian(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..4 {
            let x = self.0[3 - i];
            for j in 0..8 {
                out[i * 8 + j] = (x >> (56 - j * 8)) as u8;
            }
        }
        out
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0, 0, 0, 0]
    }

    pub fn low_u64(&self) -> u64 {
        self.0[0]
    }

    pub fn low_usize(&self) -> usize {
        self.0[0] as usize
    }

    pub fn bits_fit_usize(&self) -> bool {
        self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0 && self.0[0] <= usize::MAX as u64
    }

    pub fn overflowing_add(self, rhs: U256) -> (U256, bool) {
        let mut res = [0u64; 4];
        let mut carry = 0u128;
        for i in 0..4 {
            let sum = self.0[i] as u128 + rhs.0[i] as u128 + carry;
            res[i] = sum as u64;
            carry = sum >> 64;
        }
        (U256(res), carry != 0)
    }

    pub fn wrapping_add(self, rhs: U256) -> U256 {
        self.overflowing_add(rhs).0
    }

    pub fn overflowing_sub(self, rhs: U256) -> (U256, bool) {
        let mut res = [0u64; 4];
        let mut borrow = 0i128;
        for i in 0..4 {
            let diff = self.0[i] as i128 - rhs.0[i] as i128 - borrow;
            if diff < 0 {
                res[i] = (diff + (1i128 << 64)) as u64;
                borrow = 1;
            } else {
                res[i] = diff as u64;
                borrow = 0;
            }
        }
        (U256(res), borrow != 0)
    }

    pub fn wrapping_sub(self, rhs: U256) -> U256 {
        self.overflowing_sub(rhs).0
    }

    pub fn wrapping_mul(self, rhs: U256) -> U256 {
        let mut res = [0u64; 8];
        for i in 0..4 {
            let mut carry = 0u128;
            for j in 0..4 {
                if i + j < 8 {
                    let cur = res[i + j] as u128 + self.0[i] as u128 * rhs.0[j] as u128 + carry;
                    res[i + j] = cur as u64;
                    carry = cur >> 64;
                }
            }
            if i + 4 < 8 {
                res[i + 4] = res[i + 4].wrapping_add(carry as u64);
            }
        }
        U256([res[0], res[1], res[2], res[3]])
    }

    pub fn cmp_u(&self, other: &U256) -> std::cmp::Ordering {
        for i in (0..4).rev() {
            match self.0[i].cmp(&other.0[i]) {
                std::cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        }
        std::cmp::Ordering::Equal
    }

    pub fn lt(&self, other: &U256) -> bool {
        self.cmp_u(other) == std::cmp::Ordering::Less
    }

    pub fn bit(&self, i: usize) -> bool {
        if i >= 256 {
            return false;
        }
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }

    fn bit_len(&self) -> usize {
        for i in (0..256).rev() {
            if self.bit(i) {
                return i + 1;
            }
        }
        0
    }

    pub fn shl(self, sh: usize) -> U256 {
        if sh >= 256 {
            return U256::ZERO;
        }
        let mut out = [0u64; 4];
        let word = sh / 64;
        let bit = sh % 64;
        for i in 0..4 {
            if i + word < 4 {
                out[i + word] |= self.0[i] << bit;
                if bit > 0 && i + word + 1 < 4 {
                    out[i + word + 1] |= self.0[i] >> (64 - bit);
                }
            }
        }
        U256(out)
    }

    pub fn shr(self, sh: usize) -> U256 {
        if sh >= 256 {
            return U256::ZERO;
        }
        let mut out = [0u64; 4];
        let word = sh / 64;
        let bit = sh % 64;
        for i in 0..4 {
            if i >= word {
                out[i - word] |= self.0[i] >> bit;
                if bit > 0 && i - word >= 1 {
                    out[i - word - 1] |= self.0[i] << (64 - bit);
                }
            }
        }
        U256(out)
    }

    pub fn div_mod(self, rhs: U256) -> (U256, U256) {
        if rhs.is_zero() {
            return (U256::ZERO, U256::ZERO);
        }
        if self.lt(&rhs) {
            return (U256::ZERO, self);
        }
        let mut q = U256::ZERO;
        let mut rem = U256::ZERO;
        let n = self.bit_len();
        for i in (0..n).rev() {
            rem = rem.shl(1);
            if self.bit(i) {
                rem.0[0] |= 1;
            }
            if rem.cmp_u(&rhs) != std::cmp::Ordering::Less {
                rem = rem.wrapping_sub(rhs);
                q.0[i / 64] |= 1u64 << (i % 64);
            }
        }
        (q, rem)
    }

    pub fn and(self, rhs: U256) -> U256 {
        U256([
            self.0[0] & rhs.0[0],
            self.0[1] & rhs.0[1],
            self.0[2] & rhs.0[2],
            self.0[3] & rhs.0[3],
        ])
    }

    pub fn or(self, rhs: U256) -> U256 {
        U256([
            self.0[0] | rhs.0[0],
            self.0[1] | rhs.0[1],
            self.0[2] | rhs.0[2],
            self.0[3] | rhs.0[3],
        ])
    }

    pub fn xor(self, rhs: U256) -> U256 {
        U256([
            self.0[0] ^ rhs.0[0],
            self.0[1] ^ rhs.0[1],
            self.0[2] ^ rhs.0[2],
            self.0[3] ^ rhs.0[3],
        ])
    }

    pub fn not(self) -> U256 {
        U256([!self.0[0], !self.0[1], !self.0[2], !self.0[3]])
    }

    pub fn is_neg(&self) -> bool {
        self.bit(255)
    }

    pub fn neg(self) -> U256 {
        self.not().wrapping_add(U256::ONE)
    }
}

impl std::fmt::Debug for U256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.to_big_endian()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_overflow_wraps() {
        let max = U256([u64::MAX; 4]);
        assert_eq!(max.wrapping_add(U256::ONE), U256::ZERO);
    }

    #[test]
    fn mul_basic() {
        let a = U256::from_u64(1_000_000);
        let b = U256::from_u64(1_000_000);
        assert_eq!(a.wrapping_mul(b), U256::from_u64(1_000_000_000_000));
    }

    #[test]
    fn div_mod_basic() {
        let a = U256::from_u64(100);
        let b = U256::from_u64(7);
        let (q, r) = a.div_mod(b);
        assert_eq!(q, U256::from_u64(14));
        assert_eq!(r, U256::from_u64(2));
    }

    #[test]
    fn shifts() {
        let a = U256::from_u64(1);
        assert_eq!(a.shl(8), U256::from_u64(256));
        assert_eq!(U256::from_u64(256).shr(8), U256::from_u64(1));
        assert_eq!(a.shl(255).shr(255), U256::from_u64(1));
    }

    #[test]
    fn be_roundtrip() {
        let a = U256::from_u64(0xdead_beef);
        let bytes = a.to_big_endian();
        assert_eq!(U256::from_big_endian(&bytes), a);
    }

    #[test]
    fn big_mul_and_div() {
        let a = U256::from_big_endian(&hex::decode("0de0b6b3a7640000").unwrap());
        let (q, _) = a.div_mod(U256::from_u64(1_000_000_000));
        assert_eq!(q, U256::from_u64(1_000_000_000));
    }
}
