//! Module emulating the operations on signed fixed points with exactly 18
//! decimals as used in the Balancer smart contracts, particularly for Gyro
//! pools.

use {
    super::error::Error,
    anyhow::{Context, Result, bail},
    ethcontract::{I256, U256},
    num::{BigInt, Signed},
    std::{
        fmt::{self, Debug, Formatter},
        str::FromStr,
        sync::LazyLock,
    },
};

// Constants using BigInt for signed arithmetic
static ONE_18: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(18));
static ONE_38: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(38));
static E_19: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(10).pow(19));

static ONE_18_I256: LazyLock<I256> = LazyLock::new(|| I256::exp10(18));
static ONE_38_I256: LazyLock<I256> = LazyLock::new(|| {
    // 1e38 = 1 followed by 38 zeros
    // Since I256::exp10 might not support 38, we'll construct it manually
    let mut result = I256::from(1);
    for _ in 0..38 {
        result = result
            .checked_mul(I256::from(10))
            .expect("1e38 should fit in I256");
    }
    result
});
static ZERO_SIGNED: LazyLock<SBfp> = LazyLock::new(|| SBfp(I256::zero()));
static ONE_I256_SIGNED: LazyLock<SBfp> = LazyLock::new(|| SBfp(*ONE_18_I256));

/// Precision level for fixed-point parsing
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedPointPrecision {
    /// Standard 18-decimal precision (1e18) - for most parameters
    Standard18,
    /// Extended 38-decimal precision (1e38) - for GyroECLP high-precision
    /// parameters
    Extended38,
}

impl FixedPointPrecision {
    /// Determine precision based on GyroECLP parameter name
    /// Use this helper to identify which precision level to use for specific
    /// parameters
    pub fn for_gyro_eclp_param(param_name: &str) -> Self {
        match param_name {
            "tauAlphaX" | "tauAlphaY" | "tauBetaX" | "tauBetaY" | "u" | "v" | "w" | "z" | "dSq" => {
                Self::Extended38
            }
            "paramsAlpha" | "paramsBeta" | "paramsC" | "paramsS" | "paramsLambda" => {
                Self::Standard18
            }
            _ => {
                Self::Standard18 // Default for unknown parameters
            }
        }
    }
}

/// Signed Balancer Fixed Point - wraps I256 for signed 18-decimal fixed point
/// arithmetic Complements the unsigned Bfp type for Gyroscope pools that need
/// signed parameters
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct SBfp(I256);

impl SBfp {
    pub fn zero() -> Self {
        *ZERO_SIGNED
    }

    pub fn one() -> Self {
        *ONE_I256_SIGNED
    }

    pub fn from_wei(num: I256) -> Self {
        Self(num)
    }

    pub fn as_i256(self) -> I256 {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn is_positive(&self) -> bool {
        self.0 > I256::zero()
    }

    pub fn is_negative(&self) -> bool {
        self.0 < I256::zero()
    }

    /// Convert to BigInt for use with SignedFixedPoint operations
    pub fn to_big_int(self) -> BigInt {
        // Convert I256 to BigInt
        let mut bytes = [0u8; 32];
        self.0.to_big_endian(&mut bytes);
        if self.0 >= I256::zero() {
            BigInt::from_bytes_be(num::bigint::Sign::Plus, &bytes)
        } else {
            // For negative numbers, we need to handle two's complement
            let mut positive_bytes = [0u8; 32];
            (-self.0).to_big_endian(&mut positive_bytes);
            -BigInt::from_bytes_be(num::bigint::Sign::Plus, &positive_bytes)
        }
    }

    /// Create from BigInt (result of SignedFixedPoint operations)
    pub fn from_big_int(value: &BigInt) -> Result<Self, Error> {
        let (sign, bytes) = value.to_bytes_be();
        if bytes.len() > 32 {
            return Err(Error::MulOverflow); // Reuse existing error for overflow
        }

        let mut padded = [0u8; 32];
        let start = 32 - bytes.len();
        padded[start..].copy_from_slice(&bytes);

        let u_result = U256::from_big_endian(&padded);
        let result = I256::from_raw(u_result);
        Ok(Self(if sign == num::bigint::Sign::Minus {
            -result
        } else {
            result
        }))
    }

    /// Perform signed addition using SignedFixedPoint
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::add(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Perform signed subtraction using SignedFixedPoint
    #[allow(clippy::should_implement_trait)]
    pub fn sub(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::sub(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Perform signed multiplication with downward magnitude rounding
    pub fn mul_down_mag(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::mul_down_mag(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Perform signed multiplication with upward magnitude rounding
    pub fn mul_up_mag(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::mul_up_mag(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Perform signed division with downward magnitude rounding
    pub fn div_down_mag(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::div_down_mag(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Perform signed division with upward magnitude rounding
    pub fn div_up_mag(self, other: Self) -> Result<Self, Error> {
        let result = SignedFixedPoint::div_up_mag(&self.to_big_int(), &other.to_big_int())?;
        Self::from_big_int(&result)
    }

    /// Parse decimal string with specified precision level
    /// This preserves full mathematical precision without truncation
    pub fn from_str_with_precision(
        s: &str,
        precision: FixedPointPrecision,
    ) -> Result<Self, anyhow::Error> {
        // Handle negative sign
        let (is_negative, s) = if let Some(stripped) = s.strip_prefix('-') {
            (true, stripped)
        } else {
            (false, s)
        };

        // Get scaling factor based on precision
        let (scaling_factor, max_decimals) = match precision {
            FixedPointPrecision::Standard18 => (&*ONE_18_I256, 18),
            FixedPointPrecision::Extended38 => (&*ONE_38_I256, 38),
        };

        let mut split_dot = s.splitn(2, '.');
        let units = split_dot
            .next()
            .expect("Splitting a string slice yields at least one element");
        let decimals = split_dot.next().unwrap_or("0");

        if units.is_empty() {
            bail!("Invalid decimal representation");
        }

        // Handle high-precision decimals - preserve all digits up to max_decimals
        let processed_decimals = if decimals.len() > max_decimals {
            // For DeFi precision, we cannot truncate - this would be a logic error
            bail!(
                "Decimal precision {} exceeds maximum supported precision {} for this parameter \
                 type",
                decimals.len(),
                max_decimals
            );
        } else {
            // Pad with zeros to reach full precision
            format!("{decimals:0<width$}", width = max_decimals)
        };

        let units_value = I256::from_dec_str(units)?;
        let decimals_value = I256::from_dec_str(&processed_decimals)?;

        let mut result = units_value
            .checked_mul(*scaling_factor)
            .context("Number too large for this precision level")?
            .checked_add(decimals_value)
            .context("Number too large for this precision level")?;

        if is_negative {
            result = -result;
        }

        Ok(SBfp(result))
    }

    /// Calculate complement: ONE - x, with bounds checking
    pub fn complement(self) -> Self {
        let result = SignedFixedPoint::complement(&self.to_big_int());
        Self::from_big_int(&result).unwrap_or(Self::zero())
    }
}

impl FromStr for SBfp {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Default to Standard18 precision for backward compatibility
        // For high-precision parameters, use from_str_with_precision directly
        Self::from_str_with_precision(s, FixedPointPrecision::Standard18)
    }
}

// Enable serde deserialization from strings
impl<'de> serde::Deserialize<'de> for SBfp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        // For very high precision decimal strings (35+ digits), use Extended38
        // precision Otherwise, use Standard18 precision for backward
        // compatibility
        let precision =
            if s.contains('.') && s.split('.').nth(1).map_or(0, |decimals| decimals.len()) > 30 {
                FixedPointPrecision::Extended38
            } else {
                FixedPointPrecision::Standard18
            };

        SBfp::from_str_with_precision(&s, precision).map_err(serde::de::Error::custom)
    }
}

/// Wrapper type for high-precision (38-decimal) signed fixed point values
/// Automatically deserializes with Extended38 precision
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct HighPrecisionSBfp(pub SBfp);

impl HighPrecisionSBfp {
    pub fn inner(self) -> SBfp {
        self.0
    }
}

impl std::ops::Deref for HighPrecisionSBfp {
    type Target = SBfp;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<SBfp> for HighPrecisionSBfp {
    fn from(value: SBfp) -> Self {
        Self(value)
    }
}

impl From<HighPrecisionSBfp> for SBfp {
    fn from(value: HighPrecisionSBfp) -> Self {
        value.0
    }
}

impl<'de> serde::Deserialize<'de> for HighPrecisionSBfp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        SBfp::from_str_with_precision(&s, FixedPointPrecision::Extended38)
            .map(HighPrecisionSBfp)
            .map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for SBfp {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        let abs_value = self.0.abs();
        let sign = if self.0 < I256::zero() { "-" } else { "" };
        write!(
            formatter,
            "{}{}.{:0>18}",
            sign,
            abs_value / *ONE_18_I256,
            (abs_value % *ONE_18_I256).as_u128()
        )
    }
}

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
        if remainder == BigInt::from(0)
            || (dividend >= &BigInt::from(0)) == (divisor >= &BigInt::from(0))
        {
            quotient
        } else {
            // Different signs and remainder exists: subtract 1 to floor toward negative
            // infinity
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

        // Check for overflow: a == 0 or product // a == b (using floor division like
        // Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }
        Ok(Self::floor_div(&product, &ONE_18))
    }

    /// Multiply with downward magnitude rounding (unchecked)
    /// Equivalent to Python: mul_down_mag_u(a, b)
    pub fn mul_down_mag_u(a: &BigInt, b: &BigInt) -> BigInt {
        let product = a * b;
        let abs_result = Self::floor_div(&product.abs(), &ONE_18);
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

        // Check for overflow: a == 0 or product // a == b (using floor division like
        // Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }

        if product > BigInt::from(0) {
            Ok(Self::floor_div(&(&product - 1), &ONE_18) + 1)
        } else if product < BigInt::from(0) {
            Ok(Self::floor_div(&(&product + 1), &ONE_18) - 1)
        } else {
            Ok(BigInt::from(0))
        }
    }

    /// Multiply with upward magnitude rounding (unchecked)
    /// Equivalent to Python: mul_up_mag_u(a, b)
    pub fn mul_up_mag_u(a: &BigInt, b: &BigInt) -> BigInt {
        let product = a * b;
        if product > BigInt::from(0) {
            Self::floor_div(&(&product - 1), &ONE_18) + 1
        } else if product < BigInt::from(0) {
            Self::floor_div(&(&product + 1), &ONE_18) - 1
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
            return Err(Error::DivInternal);
        }

        Ok(Self::floor_div(&a_inflated, b))
    }

    /// Divide with downward magnitude rounding (unchecked)
    /// Equivalent to Python: div_down_mag_u(a, b)
    pub fn div_down_mag_u(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        if b == &BigInt::from(0) {
            return Err(Error::ZeroDivision);
        }

        // Python uses floor division even in "unchecked" version: abs(product) //
        // abs(b)
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
            return Err(Error::DivInternal);
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

        // Check for overflow: a == 0 or product // a == b (using floor division like
        // Python)
        if !(a == &BigInt::from(0) || Self::floor_div(&product, a) == *b) {
            return Err(Error::MulOverflow);
        }

        Ok(Self::floor_div(&product, &ONE_38))
    }

    /// Multiply with extra precision (unchecked)
    /// Equivalent to Python: mul_xp_u(a, b)
    pub fn mul_xp_u(a: &BigInt, b: &BigInt) -> BigInt {
        Self::floor_div(&(a * b), &ONE_38)
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
            return Err(Error::DivInternal);
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

    /// Multiply with extra precision, convert to normal precision with downward
    /// rounding Equivalent to Python: mul_down_xp_to_np(a, b)
    pub fn mul_down_xp_to_np(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let b1 = Self::floor_div(b, &E_19);
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
            Ok(Self::floor_div(
                &(&prod1 + Self::floor_div(&prod2, &E_19)),
                &E_19,
            ))
        } else {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &E_19) + 1), &E_19) - 1)
        }
    }

    /// Multiply with extra precision, convert to normal precision with downward
    /// rounding (unchecked) Equivalent to Python: mul_down_xp_to_np_u(a, b)
    pub fn mul_down_xp_to_np_u(a: &BigInt, b: &BigInt) -> BigInt {
        let b1 = Self::floor_div(b, &E_19);
        let b2 = b % &*E_19;
        let prod1 = a * &b1;
        let prod2 = a * &b2;

        if prod1 >= BigInt::from(0) && prod2 >= BigInt::from(0) {
            Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &E_19)), &E_19)
        } else {
            Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &E_19) + 1), &E_19) - 1
        }
    }

    /// Multiply with extra precision, convert to normal precision with upward
    /// rounding Equivalent to Python: mul_up_xp_to_np(a, b)
    pub fn mul_up_xp_to_np(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
        let b1 = Self::floor_div(b, &E_19);
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
            Ok(Self::floor_div(
                &(&prod1 + Self::floor_div(&prod2, &E_19)),
                &E_19,
            ))
        } else {
            Ok(Self::floor_div(&(&prod1 + Self::floor_div(&prod2, &E_19) - 1), &E_19) + 1)
        }
    }

    /// Multiply with extra precision, convert to normal precision with upward
    /// rounding (unchecked) Equivalent to Python: mul_up_xp_to_np_u(a, b)
    pub fn mul_up_xp_to_np_u(a: &BigInt, b: &BigInt) -> BigInt {
        let b1 = Self::floor_div(b, &E_19);
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
            trunc_div(&(&prod1 + trunc_div(&prod2, &E_19)), &E_19)
        } else {
            trunc_div(&(&prod1 + trunc_div(&prod2, &E_19) - 1), &E_19) + 1
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

    #[test]
    fn test_from_str_with_precision_standard() {
        // Test standard 18-decimal precision
        let value =
            SBfp::from_str_with_precision("0.707106781186547524", FixedPointPrecision::Standard18)
                .unwrap();
        let expected = SBfp::from_wei(I256::from_dec_str("707106781186547524").unwrap());
        assert_eq!(value, expected);

        // Test negative value
        let value = SBfp::from_str_with_precision("-0.5", FixedPointPrecision::Standard18).unwrap();
        let expected = SBfp::from_wei(-I256::from_dec_str("500000000000000000").unwrap());
        assert_eq!(value, expected);
    }

    #[test]
    fn test_from_str_with_precision_extended() {
        // Test 38-decimal precision with high-precision value
        let high_precision_str = "-0.17378533390904767196396190604716688";
        let value =
            SBfp::from_str_with_precision(high_precision_str, FixedPointPrecision::Extended38)
                .unwrap();

        // The expected value should be the decimal scaled by 1e38
        // -0.17378533390904767196396190604716688 * 1e38 =
        // -17378533390904767196396190604716688000
        let expected_str = "17378533390904767196396190604716688000";
        let expected = SBfp::from_wei(-I256::from_dec_str(expected_str).unwrap());
        assert_eq!(value, expected);
    }

    #[test]
    fn test_precision_limits() {
        // Test that exceeding precision limits fails appropriately
        let too_high_precision = "0.123456789012345678901234567890123456789"; // 39 digits

        // Should fail for standard precision
        assert!(
            SBfp::from_str_with_precision(too_high_precision, FixedPointPrecision::Standard18)
                .is_err()
        );

        // Should also fail for extended precision (39 > 38)
        assert!(
            SBfp::from_str_with_precision(too_high_precision, FixedPointPrecision::Extended38)
                .is_err()
        );
    }

    #[test]
    fn test_backward_compatibility() {
        // Test that the standard FromStr still works for 18-decimal values
        let value: SBfp = "0.5".parse().unwrap();
        let expected = SBfp::from_wei(I256::from_dec_str("500000000000000000").unwrap());
        assert_eq!(value, expected);
    }

    #[test]
    fn test_precision_helper() {
        // Test precision selection helper
        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("tauAlphaX"),
            FixedPointPrecision::Extended38
        );
        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("u"),
            FixedPointPrecision::Extended38
        );
        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("dSq"),
            FixedPointPrecision::Extended38
        );

        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("paramsAlpha"),
            FixedPointPrecision::Standard18
        );
        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("paramsC"),
            FixedPointPrecision::Standard18
        );
        assert_eq!(
            FixedPointPrecision::for_gyro_eclp_param("unknown_param"),
            FixedPointPrecision::Standard18
        );
    }

    #[test]
    fn test_automatic_precision_detection() {
        // Test that the Deserialize implementation automatically detects precision

        // Standard precision values (<=30 decimals) should use 18-decimal scaling
        let standard_json = serde_json::json!("0.707106781186547524");
        let standard_value: SBfp = serde_json::from_value(standard_json).unwrap();
        let expected_standard =
            SBfp::from_str_with_precision("0.707106781186547524", FixedPointPrecision::Standard18)
                .unwrap();
        assert_eq!(standard_value, expected_standard);

        // High precision values (>30 decimals) should use 38-decimal scaling
        let high_precision_json = serde_json::json!("-0.17378533390904767196396190604716688");
        let high_precision_value: SBfp = serde_json::from_value(high_precision_json).unwrap();
        let expected_high_precision = SBfp::from_str_with_precision(
            "-0.17378533390904767196396190604716688",
            FixedPointPrecision::Extended38,
        )
        .unwrap();
        assert_eq!(high_precision_value, expected_high_precision);

        // Verify they're different (high precision should preserve more information)
        assert_ne!(
            standard_value.to_big_int().abs(),
            high_precision_value.to_big_int().abs()
        );

        // Integer values should use standard precision
        let integer_json = serde_json::json!("42");
        let integer_value: SBfp = serde_json::from_value(integer_json).unwrap();
        let expected_integer =
            SBfp::from_str_with_precision("42", FixedPointPrecision::Standard18).unwrap();
        assert_eq!(integer_value, expected_integer);
    }
}
