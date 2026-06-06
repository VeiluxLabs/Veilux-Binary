use crate::u256::U256;

fn modulus() -> U256 {
    U256::from_big_endian(&[
        0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58,
        0x5d, 0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c,
        0xfd, 0x47,
    ])
}

fn addmod(a: U256, b: U256, p: U256) -> U256 {
    let (sum, carry) = a.overflowing_add(b);
    if carry || sum.cmp_u(&p) != std::cmp::Ordering::Less {
        sum.wrapping_sub(p)
    } else {
        sum
    }
}

fn submod(a: U256, b: U256, p: U256) -> U256 {
    if a.cmp_u(&b) == std::cmp::Ordering::Less {
        a.wrapping_add(p).wrapping_sub(b)
    } else {
        a.wrapping_sub(b)
    }
}

fn mulmod(a: U256, b: U256, p: U256) -> U256 {
    let mut result = U256::ZERO;
    let mut base = a.div_mod(p).1;
    let mut exp = b;
    while !exp.is_zero() {
        if exp.bit(0) {
            result = addmod(result, base, p);
        }
        base = addmod(base, base, p);
        exp = exp.shr(1);
    }
    result
}

fn powmod(a: U256, e: U256, p: U256) -> U256 {
    let mut result = U256::ONE;
    let mut base = a.div_mod(p).1;
    let mut exp = e;
    while !exp.is_zero() {
        if exp.bit(0) {
            result = mulmod(result, base, p);
        }
        base = mulmod(base, base, p);
        exp = exp.shr(1);
    }
    result
}

fn invmod(a: U256, p: U256) -> U256 {
    let two = U256::from_u64(2);
    powmod(a, p.wrapping_sub(two), p)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct G1 {
    pub x: U256,
    pub y: U256,
    pub infinity: bool,
}

impl G1 {
    pub fn infinity() -> Self {
        G1 {
            x: U256::ZERO,
            y: U256::ZERO,
            infinity: true,
        }
    }

    pub fn new(x: U256, y: U256) -> Self {
        if x.is_zero() && y.is_zero() {
            return G1::infinity();
        }
        G1 {
            x,
            y,
            infinity: false,
        }
    }

    pub fn is_on_curve(&self) -> bool {
        if self.infinity {
            return true;
        }
        let p = modulus();
        let y2 = mulmod(self.y, self.y, p);
        let x3 = mulmod(mulmod(self.x, self.x, p), self.x, p);
        let rhs = addmod(x3, U256::from_u64(3), p);
        y2 == rhs
    }

    pub fn double(&self) -> G1 {
        if self.infinity || self.y.is_zero() {
            return G1::infinity();
        }
        let p = modulus();
        let three = U256::from_u64(3);
        let two = U256::from_u64(2);
        let num = mulmod(three, mulmod(self.x, self.x, p), p);
        let den = invmod(mulmod(two, self.y, p), p);
        let lambda = mulmod(num, den, p);
        let x3 = submod(mulmod(lambda, lambda, p), mulmod(two, self.x, p), p);
        let y3 = submod(mulmod(lambda, submod(self.x, x3, p), p), self.y, p);
        G1::new(x3, y3)
    }

    pub fn add(&self, other: &G1) -> G1 {
        if self.infinity {
            return *other;
        }
        if other.infinity {
            return *self;
        }
        let p = modulus();
        if self.x == other.x {
            if self.y == other.y {
                return self.double();
            }
            return G1::infinity();
        }
        let num = submod(other.y, self.y, p);
        let den = invmod(submod(other.x, self.x, p), p);
        let lambda = mulmod(num, den, p);
        let x3 = submod(submod(mulmod(lambda, lambda, p), self.x, p), other.x, p);
        let y3 = submod(mulmod(lambda, submod(self.x, x3, p), p), self.y, p);
        G1::new(x3, y3)
    }

    pub fn mul(&self, scalar: U256) -> G1 {
        let mut result = G1::infinity();
        let mut base = *self;
        let mut k = scalar;
        while !k.is_zero() {
            if k.bit(0) {
                result = result.add(&base);
            }
            base = base.double();
            k = k.shr(1);
        }
        result
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        if !self.infinity {
            out[..32].copy_from_slice(&self.x.to_big_endian());
            out[32..].copy_from_slice(&self.y.to_big_endian());
        }
        out
    }
}

fn read_point(input: &[u8], offset: usize) -> Option<G1> {
    let mut buf = [0u8; 64];
    let end = (offset + 64).min(input.len());
    if offset < end {
        buf[..end - offset].copy_from_slice(&input[offset..end]);
    }
    let x = U256::from_big_endian(&buf[..32]);
    let y = U256::from_big_endian(&buf[32..]);
    let pt = G1::new(x, y);
    if pt.is_on_curve() {
        Some(pt)
    } else {
        None
    }
}

pub fn ec_add(input: &[u8]) -> Option<Vec<u8>> {
    let a = read_point(input, 0)?;
    let b = read_point(input, 64)?;
    Some(a.add(&b).to_bytes())
}

pub fn ec_mul(input: &[u8]) -> Option<Vec<u8>> {
    let a = read_point(input, 0)?;
    let mut sbuf = [0u8; 32];
    let start = 64.min(input.len());
    let end = 96.min(input.len());
    if start < end {
        sbuf[..end - start].copy_from_slice(&input[start..end]);
    }
    let scalar = U256::from_big_endian(&sbuf);
    Some(a.mul(scalar).to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator() -> G1 {
        G1::new(U256::from_u64(1), U256::from_u64(2))
    }

    #[test]
    fn generator_is_on_curve() {
        assert!(generator().is_on_curve());
    }

    #[test]
    fn double_matches_add_self() {
        let g = generator();
        let d = g.double();
        let a = g.add(&g);
        assert_eq!(d.x, a.x);
        assert_eq!(d.y, a.y);
        assert!(d.is_on_curve());
    }

    #[test]
    fn two_g_is_on_curve_and_consistent() {
        let g = generator();
        let two_g = g.mul(U256::from_u64(2));
        assert_eq!(two_g, g.add(&g));
        assert!(two_g.is_on_curve());
    }

    #[test]
    fn scalar_mul_distributes() {
        let g = generator();
        let five = g.mul(U256::from_u64(5));
        let two = g.mul(U256::from_u64(2));
        let three = g.mul(U256::from_u64(3));
        assert_eq!(five, two.add(&three));
        assert!(five.is_on_curve());
    }

    #[test]
    fn add_inverse_is_infinity() {
        let g = generator();
        let p = modulus();
        let neg = G1::new(g.x, submod(U256::ZERO, g.y, p));
        assert!(g.add(&neg).infinity);
    }

    #[test]
    fn ec_add_precompile_roundtrip() {
        let g = generator();
        let mut input = vec![0u8; 128];
        input[..64].copy_from_slice(&g.to_bytes());
        input[64..].copy_from_slice(&g.to_bytes());
        let out = ec_add(&input).unwrap();
        assert_eq!(out, g.double().to_bytes());
    }
}
