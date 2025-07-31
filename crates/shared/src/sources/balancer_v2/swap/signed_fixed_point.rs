//! Module emulating the operations on signed fixed points with exactly 18 decimals
//! as used in the Balancer smart contracts, particularly for Gyro pools.
//! This mirrors the implementation from:
//! https://github.com/balancer-labs/balancer-maths/blob/main/python/src/pools/gyro/signed_fixed_point.py

use {
    super::error::Error,
    num::{BigInt, Signed},
    std::sync::LazyLock,
};

// Constants using BigInt for signed arithmetic
static ONE_18: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(18));
static ONE_38: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(38));
static E_19: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(19));

pub struct SignedFixedPoint;

impl SignedFixedPoint {
    /// ONE = 1e18 (18 decimal places)
    pub fn one() -> BigInt {
        ONE_18.clone()
    }

    /// Floor division that matches Python's // operator behavior
    /// Rounds toward negative infinity (down)
    fn floor_div(dividend: &BigInt, divisor: &BigInt) -> BigInt {
        let quotient = dividend / divisor;
        let remainder = dividend % divisor;
        
        // If there's no remainder, or both have same sign, return regular division
        if remainder == BigInt::from(0) || (dividend >= &BigInt::from(0)) == (divisor >= &BigInt::from(0)) {
            quotient
        } else {
            // Different signs and remainder exists: subtract 1 to floor toward negative infinity
            quotient - 1
        }
    }

    /// ONE_XP = 1e38 (38 decimal places for extra precision)
    pub fn one_xp() -> BigInt {
        ONE_38.clone()
    }

    /// Signed addition with overflow checking
    /// Equivalent to Python: add(a, b)
    pub fn add(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let c = a + b;
        
        // Check for overflow: if b >= 0, then c >= a; if b < 0, then c < a
        if !((b >= &BigInt::from(0) && &c >= a) || (b < &BigInt::from(0) && &c < a)) {
            return Err(Error::AddOverflow);
        }
        Ok(c)
    }

    /// Add with magnitude - if a > 0, add; else subtract
    /// Equivalent to Python: add_mag(a, b)
    pub fn add_mag(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if a > &BigInt::from(0) {
            Self::add(a, b)
        } else {
            Self::sub(a, b)
        }
    }

    /// Signed subtraction with overflow checking
    /// Equivalent to Python: sub(a, b)
    pub fn sub(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let c = a - b;
        
        // Check for overflow: if b <= 0, then c >= a; if b > 0, then c < a
        if !((b <= &BigInt::from(0) && &c >= a) || (b > &BigInt::from(0) && &c < a)) {
            return Err(Error::SubOverflow);
        }
        Ok(c)
    }

    /// Multiply with downward magnitude rounding
    /// Equivalent to Python: mul_down_mag(a, b)
    pub fn mul_down_mag(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let product = a * b;
        
        // Check for overflow: a == 0 or product // a == b (using floor division like Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }
        Ok(Self::floor_div(&product, &*ONE_18))
    }

    /// Multiply with downward magnitude rounding (unchecked)
    /// Equivalent to Python: mul_down_mag_u(a, b)
    pub fn mul_down_mag_u(a: &BigInt, b: &BigInt) -> BigInt {
        let product = a * b;
        let abs_result = Self::floor_div(&product.abs(), &*ONE_18);
        if product < BigInt::from(0) {
            -abs_result
        } else {
            abs_result
        }
    }

    /// Multiply with upward magnitude rounding
    /// Equivalent to Python: mul_up_mag(a, b)
    pub fn mul_up_mag(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let product = a * b;
        
        // Check for overflow: a == 0 or product // a == b (using floor division like Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }

        if product > BigInt::from(0) {
            Ok(Self::floor_div(&(&product - 1), &*ONE_18) + 1)
        } else if product < BigInt::from(0) {
            Ok(Self::floor_div(&(&product + 1), &*ONE_18) - 1)
        } else {
            Ok(BigInt::from(0))
        }
    }

    /// Multiply with upward magnitude rounding (unchecked)
    /// Equivalent to Python: mul_up_mag_u(a, b)
    pub fn mul_up_mag_u(a: &BigInt, b: &BigInt) -> BigInt {
        let product = a * b;
        if product > BigInt::from(0) {
            Self::floor_div(&(&product - 1), &*ONE_18) + 1
        } else if product < BigInt::from(0) {
            Self::floor_div(&(&product + 1), &*ONE_18) - 1
        } else {
            BigInt::from(0)
        }
    }

    /// Divide with downward magnitude rounding
    /// Equivalent to Python: div_down_mag(a, b)
    pub fn div_down_mag(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }
        if a == &BigInt::from(0) {
            return Ok(BigInt::from(0));
        }

        let a_inflated = a * &*ONE_18;
        if Self::floor_div(&a_inflated, a) != *ONE_18 {
            return Err(Error::DivInterval);
        }

        Ok(Self::floor_div(&a_inflated, b))
    }

    /// Divide with downward magnitude rounding (unchecked)
    /// Equivalent to Python: div_down_mag_u(a, b)
    pub fn div_down_mag_u(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }

        // Python uses floor division even in "unchecked" version: abs(product) // abs(b)
        let product = a * &*ONE_18;
        let abs_result = Self::floor_div(&product.abs(), &b.abs());
        // Apply the correct sign
        Ok(if (product < BigInt::from(0)) != (b < &BigInt::from(0)) {
            -abs_result
        } else {
            abs_result
        })
    }

    /// Divide with upward magnitude rounding
    /// Equivalent to Python: div_up_mag(a, b)
    pub fn div_up_mag(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }
        if a == &BigInt::from(0) {
            return Ok(BigInt::from(0));
        }

        let mut local_a = a.clone();
        let mut local_b = b.clone();
        if b < &BigInt::from(0) {
            local_b = -b;
            local_a = -a;
        }

        let a_inflated = &local_a * &*ONE_18;
        if Self::floor_div(&a_inflated, &local_a) != *ONE_18 {
            return Err(Error::DivInterval);
        }

        if a_inflated > BigInt::from(0) {
            Ok(Self::floor_div(&(&a_inflated - 1), &local_b) + 1)
        } else {
            Ok(Self::floor_div(&(&a_inflated + 1), &local_b) - 1)
        }
    }

    /// Divide with upward magnitude rounding (unchecked)
    /// Equivalent to Python: div_up_mag_u(a, b)
    pub fn div_up_mag_u(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }
        if a == &BigInt::from(0) {
            return Ok(BigInt::from(0));
        }

        let mut local_a = a.clone();
        let mut local_b = b.clone();
        if b < &BigInt::from(0) {
            local_b = -b;
            local_a = -a;
        }

        if local_a > BigInt::from(0) {
            Ok(Self::floor_div(&(&local_a * &*ONE_18 - 1), &local_b) + 1)
        } else {
            Ok(Self::floor_div(&(&local_a * &*ONE_18 + 1), &local_b) - 1)
        }
    }

    /// Multiply with extra precision
    /// Equivalent to Python: mul_xp(a, b)
    pub fn mul_xp(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let product = a * b;
        
        // Check for overflow: a == 0 or product // a == b (using floor division like Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }
        
        Ok(Self::floor_div(&product, &*ONE_38))
    }

    /// Multiply with extra precision (unchecked)
    /// Equivalent to Python: mul_xp_u(a, b)
    pub fn mul_xp_u(a: &BigInt, b: &BigInt) -> BigInt {
        Self::floor_div(&(a * b), &*ONE_38)
    }

    /// Divide with extra precision
    /// Equivalent to Python: div_xp(a, b)
    pub fn div_xp(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }
        if a == &BigInt::from(0) {
            return Ok(BigInt::from(0));
        }

        let a_inflated = a * &*ONE_38;
        if Self::floor_div(&a_inflated, a) != *ONE_38 {
            return Err(Error::DivInterval);
        }

        Ok(Self::floor_div(&a_inflated, b))
    }

    /// Divide with extra precision (unchecked)
    /// Equivalent to Python: div_xp_u(a, b)
    pub fn div_xp_u(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }
        Ok(Self::floor_div(&(a * &*ONE_38), b))
    }

    /// Multiply with extra precision, convert to normal precision with downward rounding
    /// Equivalent to Python: mul_down_xp_to_np(a, b)
    pub fn mul_down_xp_to_np(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let b1 = Self::floor_div(b, &*E_19);
        let prod1 = a * &b1;
        if !(a == &BigInt::from(0) || Self::floor_div(&prod1, a) == b1) {
            return Err(Error::MulOverflow);
        }
        
        let b2 = b % &*E_19;
        let prod2 = a * &b2;
        if !(a == &BigInt::from(0) || Self::floor_div(&prod2, a) == b2) {
            return Err(Error::MulOverflow);
        }

        if prod1 >= BigInt::from(0) && prod2 >= BigInt::from(0) {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19)), &*E_19))
        } else {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19) + 1), &*E_19) - 1)
        }
    }

    /// Multiply with extra precision, convert to normal precision with downward rounding (unchecked)
    /// Equivalent to Python: mul_down_xp_to_np_u(a, b)
    pub fn mul_down_xp_to_np_u(a: &BigInt, b: &BigInt) -> BigInt {
        let b1 = Self::floor_div(b, &*E_19);
        let b2 = b % &*E_19;
        let prod1 = a * &b1;
        let prod2 = a * &b2;
        
        if prod1 >= BigInt::from(0) && prod2 >= BigInt::from(0) {
            Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19)), &*E_19)
        } else {
            Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19) + 1), &*E_19) - 1
        }
    }

    /// Multiply with extra precision, convert to normal precision with upward rounding
    /// Equivalent to Python: mul_up_xp_to_np(a, b)
    pub fn mul_up_xp_to_np(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let b1 = Self::floor_div(b, &*E_19);
        let prod1 = a * &b1;
        if !(a == &BigInt::from(0) || Self::floor_div(&prod1, a) == b1) {
            return Err(Error::MulOverflow);
        }
        
        let b2 = b % &*E_19;
        let prod2 = a * &b2;
        if !(a == &BigInt::from(0) || Self::floor_div(&prod2, a) == b2) {
            return Err(Error::MulOverflow);
        }

        if prod1 <= BigInt::from(0) && prod2 <= BigInt::from(0) {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19)), &*E_19))
        } else {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &*E_19) - 1), &*E_19) + 1)
        }
    }

    /// Multiply with extra precision, convert to normal precision with upward rounding (unchecked)
    /// Equivalent to Python: mul_up_xp_to_np_u(a, b)
    pub fn mul_up_xp_to_np_u(a: &BigInt, b: &BigInt) -> BigInt {
        let b1 = Self::floor_div(b, &*E_19);
        let b2 = b % &*E_19;
        let prod1 = a * &b1;
        let prod2 = a * &b2;

        // Python's trunc_div function - still uses floor division on absolute values!
        fn trunc_div(x: &BigInt, y: &BigInt) -> BigInt {
            let result = SignedFixedPoint::floor_div(&x.abs(), &y.abs());
            if (x < &BigInt::from(0)) != (y < &BigInt::from(0)) {
                -result
            } else {
                result
            }
        }

        if prod1 <= BigInt::from(0) && prod2 <= BigInt::from(0) {
            trunc_div(&(&prod1 + trunc_div(&prod2, &*E_19)), &*E_19)
        } else {
            trunc_div(&(&prod1 + trunc_div(&prod2, &*E_19) - 1), &*E_19) + 1
        }
    }

    /// Calculate complement: ONE - x, with bounds checking
    /// Equivalent to Python: complement(x)
    pub fn complement(x: &BigInt) -> BigInt {
        if x >= &*ONE_18 || x <= &BigInt::from(0) {
            BigInt::from(0)
        } else {
            &*ONE_18 - x
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        let one = SignedFixedPoint::one();
        let one_xp = SignedFixedPoint::one_xp();
        
        assert_eq!(one, BigInt::from(10).pow(18));
        assert_eq!(one_xp, BigInt::from(10).pow(38));
    }

    #[test]
    fn test_add() {
        let a = BigInt::from(100) * &*ONE_18;
        let b = BigInt::from(50) * &*ONE_18;
        let result = SignedFixedPoint::add(&a, &b).unwrap();
        assert_eq!(result, BigInt::from(150) * &*ONE_18);
    }

    #[test]
    fn test_sub() {
        let a = BigInt::from(100) * &*ONE_18;
        let b = BigInt::from(50) * &*ONE_18;
        let result = SignedFixedPoint::sub(&a, &b).unwrap();
        assert_eq!(result, BigInt::from(50) * &*ONE_18);
    }

    #[test]
    fn test_mul_down_mag_u() {
        let a = BigInt::from(2) * &*ONE_18;
        let b = BigInt::from(3) * &*ONE_18;
        let result = SignedFixedPoint::mul_down_mag_u(&a, &b);
        assert_eq!(result, BigInt::from(6) * &*ONE_18);
    }

    #[test]
    fn test_complement() {
        let half = &*ONE_18 / 2;
        let result = SignedFixedPoint::complement(&half);
        assert_eq!(result, half);
        
        let zero = BigInt::from(0);
        let result = SignedFixedPoint::complement(&zero);
        assert_eq!(result, BigInt::from(0));
        
        let one = &*ONE_18;
        let result = SignedFixedPoint::complement(one);
        assert_eq!(result, BigInt::from(0));
    }
}
