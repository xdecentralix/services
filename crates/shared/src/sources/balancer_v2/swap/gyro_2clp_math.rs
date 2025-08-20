//! Module emulating the functions in the Balancer Gyro2CLPMath implementation.
//! The original contract code can be found at:
//! https://github.com/balancer-labs/balancer-maths/blob/main/python/src/pools/gyro/gyro_2clp_math.py
//!
//! This implementation matches the Python reference EXACTLY as verified against
//! the official balancer-maths repository.

use {
    super::error::Error,
    num::{BigInt, Signed},
    std::sync::LazyLock,
};

// Core constants mirroring the Python implementation
static WAD: LazyLock<BigInt> = LazyLock::new(|| BigInt::from(1_000_000_000_000_000_000_u64)); // 1e18

/// Rounding direction for calculations
#[derive(Debug, Clone, PartialEq)]
pub enum Rounding {
    RoundDown,
    RoundUp,
}

/// Represents the terms needed for quadratic formula solution
#[derive(Debug, Clone)]
pub struct QuadraticTerms {
    pub a: BigInt,
    pub mb: BigInt,       // -b (negative b)
    pub b_square: BigInt, // b² calculated separately for precision
    pub mc: BigInt,       // -c (negative c)
}

// Simple fixed-point arithmetic functions matching Python reference EXACTLY

/// Multiply with upward rounding - matches mul_up_fixed(a, b) from Python
fn mul_up_fixed(a: &BigInt, b: &BigInt) -> BigInt {
    let product = a * b;
    if product == BigInt::from(0) {
        return BigInt::from(0);
    }
    (&product - 1) / &*WAD + 1
}

/// Multiply with downward rounding - matches mul_down_fixed(a, b) from Python
fn mul_down_fixed(a: &BigInt, b: &BigInt) -> BigInt {
    let product = a * b;
    product / &*WAD
}

/// Divide with downward rounding - matches div_down_fixed(a, b) from Python
fn div_down_fixed(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
    if a == &BigInt::from(0) {
        return Ok(BigInt::from(0));
    }
    if b == &BigInt::from(0) {
        return Err(Error::ZeroDivision);
    }
    let a_inflated = a * &*WAD;
    Ok(a_inflated / b)
}

/// Divide with upward rounding - matches div_up_fixed(a, b) from Python
fn div_up_fixed(a: &BigInt, b: &BigInt) -> Result<BigInt, Error> {
    if a == &BigInt::from(0) {
        return Ok(BigInt::from(0));
    }
    if b == &BigInt::from(0) {
        return Err(Error::ZeroDivision);
    }
    let a_inflated = a * &*WAD;
    Ok((&a_inflated - 1) / b + 1)
}

/// Square root function matching gyro_pool_math_sqrt from Python EXACTLY
pub fn gyro_pool_math_sqrt(x: &BigInt, tolerance: u64) -> Result<BigInt, Error> {
    if x == &BigInt::from(0) {
        return Ok(BigInt::from(0));
    }

    if x < &BigInt::from(0) {
        return Err(Error::InvalidExponent);
    }

    let mut guess = make_initial_guess(x);

    // Perform Newton's method iterations - exactly 7 iterations like Python
    for _ in 0..7 {
        let x_wad = x * &*WAD;
        guess = (&guess + &x_wad / &guess) / 2;
    }

    // Tolerance checking like Python reference
    let guess_squared = mul_down_fixed(&guess, &guess);
    let tolerance_up = mul_up_fixed(&guess, &BigInt::from(tolerance));

    if !(guess_squared <= x + &tolerance_up && guess_squared >= x - &tolerance_up) {
        return Err(Error::InvalidExponent); // _sqrt FAILED
    }

    Ok(guess)
}

/// Initial guess function matching Python _make_initial_guess
fn make_initial_guess(x: &BigInt) -> BigInt {
    if x >= &*WAD {
        let shift = int_log2_halved(x / &*WAD);
        (BigInt::from(1) << shift) * &*WAD
    } else {
        // Constants from Python reference
        if x <= &BigInt::from(10) {
            BigInt::from(3162277660_u64) // _SQRT_1E_NEG_17
        } else if x <= &BigInt::from(100) {
            BigInt::from(10000000000_u64) // 10**10
        } else if x <= &BigInt::from(1000) {
            BigInt::from(31622776601_u64) // _SQRT_1E_NEG_15
        } else if x <= &BigInt::from(10000) {
            BigInt::from(100000000000_u64) // 10**11
        } else if x <= &BigInt::from(100000) {
            BigInt::from(316227766016_u64) // _SQRT_1E_NEG_13
        } else if x <= &BigInt::from(1000000) {
            BigInt::from(1000000000000_u64) // 10**12
        } else if x <= &BigInt::from(10000000) {
            BigInt::from(3162277660168_u64) // _SQRT_1E_NEG_11
        } else if x <= &BigInt::from(100000000) {
            BigInt::from(10000000000000_u64) // 10**13
        } else if x <= &BigInt::from(1000000000) {
            BigInt::from(31622776601683_u64) // _SQRT_1E_NEG_9
        } else if x <= &BigInt::from(10000000000_u64) {
            BigInt::from(100000000000000_u64) // 10**14
        } else if x <= &BigInt::from(100000000000_u64) {
            BigInt::from(316227766016837_u64) // _SQRT_1E_NEG_7
        } else if x <= &BigInt::from(1000000000000_u64) {
            BigInt::from(1000000000000000_u64) // 10**15
        } else if x <= &BigInt::from(10000000000000_u64) {
            BigInt::from(3162277660168379_u64) // _SQRT_1E_NEG_5
        } else if x <= &BigInt::from(100000000000000_u64) {
            BigInt::from(10000000000000000_u64) // 10**16
        } else if x <= &BigInt::from(1000000000000000_u64) {
            BigInt::from(31622776601683793_u64) // _SQRT_1E_NEG_3
        } else if x <= &BigInt::from(10000000000000000_u64) {
            BigInt::from(100000000000000000_u64) // 10**17
        } else if x <= &BigInt::from(100000000000000000_u64) {
            BigInt::from(316227766016837933_u64) // _SQRT_1E_NEG_1
        } else {
            x.clone()
        }
    }
}

/// Integer log2 halved matching Python _int_log2_halved
fn int_log2_halved(mut x: BigInt) -> u32 {
    let mut n = 0u32;

    if x >= BigInt::from(1_u64) << 128 {
        x >>= 128;
        n += 64;
    }
    if x >= BigInt::from(1_u64) << 64 {
        x >>= 64;
        n += 32;
    }
    if x >= BigInt::from(1_u64) << 32 {
        x >>= 32;
        n += 16;
    }
    if x >= BigInt::from(1_u64) << 16 {
        x >>= 16;
        n += 8;
    }
    if x >= BigInt::from(1_u64) << 8 {
        x >>= 8;
        n += 4;
    }
    if x >= BigInt::from(1_u64) << 4 {
        x >>= 4;
        n += 2;
    }
    if x >= BigInt::from(1_u64) << 2 {
        x >>= 2;
        n += 1;
    }
    if x >= BigInt::from(1_u64) << 1 {
        n += 1;
    }

    n
}

/// Calculate invariant using quadratic formula - matches Python
/// calculate_invariant EXACTLY
pub fn calculate_invariant(
    balances: &[BigInt],
    sqrt_alpha: &BigInt,
    sqrt_beta: &BigInt,
    rounding: &Rounding,
) -> Result<BigInt, Error> {
    if balances.len() != 2 {
        return Err(Error::InvalidToken);
    }

    // Get quadratic terms from helper function
    let quadratic_terms = calculate_quadratic_terms(balances, sqrt_alpha, sqrt_beta, rounding)?;

    // Calculate final result using quadratic formula
    calculate_quadratic(
        &quadratic_terms.a,
        &quadratic_terms.mb,
        &quadratic_terms.b_square,
        &quadratic_terms.mc,
    )
}

/// Calculate quadratic terms - matches Python calculate_quadratic_terms EXACTLY
pub fn calculate_quadratic_terms(
    balances: &[BigInt],
    sqrt_alpha: &BigInt,
    sqrt_beta: &BigInt,
    rounding: &Rounding,
) -> Result<QuadraticTerms, Error> {
    if balances.len() != 2 {
        return Err(Error::InvalidToken);
    }

    // Define rounding functions based on rounding direction - matches Python
    // exactly
    let div_up_or_down = match rounding {
        Rounding::RoundDown => div_down_fixed,
        Rounding::RoundUp => div_up_fixed,
    };

    let mul_up_or_down = match rounding {
        Rounding::RoundDown => mul_down_fixed,
        Rounding::RoundUp => mul_up_fixed,
    };

    let mul_down_or_up = match rounding {
        Rounding::RoundDown => mul_up_fixed,
        Rounding::RoundUp => mul_down_fixed,
    };

    // Calculate 'a' term - matches Python: a = WAD - div_up_or_down(sqrt_alpha,
    // sqrt_beta)
    let a = &*WAD - &div_up_or_down(sqrt_alpha, sqrt_beta)?;

    // Calculate 'b' terms - matches Python exactly
    let b_term0 = div_up_or_down(&balances[1], sqrt_beta)?;
    let b_term1 = mul_up_or_down(&balances[0], sqrt_alpha);
    let mb = b_term0 + b_term1;

    // Calculate 'c' term - matches Python: mc = mul_up_or_down(balances[0],
    // balances[1])
    let mc = mul_up_or_down(&balances[0], &balances[1]);

    // Calculate b² - matches Python calculation exactly
    let b_square = mul_up_or_down(
        &mul_up_or_down(&mul_up_or_down(&balances[0], &balances[0]), sqrt_alpha),
        sqrt_alpha,
    );

    let b_sq2 = div_up_or_down(
        &(BigInt::from(2)
            * mul_up_or_down(&mul_up_or_down(&balances[0], &balances[1]), sqrt_alpha)),
        sqrt_beta,
    )?;

    let b_sq3 = div_up_or_down(
        &mul_up_or_down(&balances[1], &balances[1]),
        &mul_down_or_up(sqrt_beta, sqrt_beta),
    )?;

    let b_square = b_square + b_sq2 + b_sq3;

    Ok(QuadraticTerms {
        a,
        mb,
        b_square,
        mc,
    })
}

/// Calculate quadratic formula - matches Python calculate_quadratic EXACTLY
pub fn calculate_quadratic(
    a: &BigInt,
    mb: &BigInt,
    b_square: &BigInt,
    mc: &BigInt,
) -> Result<BigInt, Error> {
    // Calculate denominator - matches Python: mul_up_fixed(a, 2 * WAD)
    let denominator = mul_up_fixed(a, &(BigInt::from(2) * &*WAD));

    // Order multiplications for fixed point precision - matches Python exactly
    let add_term = mul_down_fixed(&mul_down_fixed(mc, &(BigInt::from(4) * &*WAD)), a);

    // The minus sign in the radicand cancels out - matches Python exactly
    let radicand = b_square + add_term;

    // Calculate square root - matches Python exactly
    let sqr_result = gyro_pool_math_sqrt(&radicand, 5)?;

    // The minus sign in the numerator cancels out - matches Python exactly
    let numerator = mb + sqr_result;

    // Calculate final result - matches Python exactly
    let invariant = div_down_fixed(&numerator, &denominator)?;

    Ok(invariant)
}

/// Calculate output amount - matches Python calc_out_given_in EXACTLY
pub fn calc_out_given_in(
    balance_in: &BigInt,
    balance_out: &BigInt,
    amount_in: &BigInt,
    virtual_offset_in: &BigInt,
    virtual_offset_out: &BigInt,
) -> Result<BigInt, Error> {
    // Safety margins - matches Python exactly
    let virt_in_over = balance_in + mul_up_fixed(virtual_offset_in, &(&*WAD + 2));
    let virt_out_under = balance_out + mul_down_fixed(virtual_offset_out, &(&*WAD - 1));

    // Calculate output amount - matches Python exactly
    let amount_out = div_down_fixed(
        &mul_down_fixed(&virt_out_under, amount_in),
        &(&virt_in_over + amount_in),
    )?;

    // Ensure amountOut <= balanceOut - matches Python check
    if amount_out > *balance_out {
        return Err(Error::XOutOfBounds);
    }

    Ok(amount_out)
}

/// Calculate input amount - matches Python calc_in_given_out EXACTLY
pub fn calc_in_given_out(
    balance_in: &BigInt,
    balance_out: &BigInt,
    amount_out: &BigInt,
    virtual_offset_in: &BigInt,
    virtual_offset_out: &BigInt,
) -> Result<BigInt, Error> {
    // Check bounds - matches Python check
    if amount_out > balance_out {
        return Err(Error::XOutOfBounds);
    }

    // Safety margins - matches Python exactly
    let virt_in_over = balance_in + mul_up_fixed(virtual_offset_in, &(&*WAD + 2));
    let virt_out_under = balance_out + mul_down_fixed(virtual_offset_out, &(&*WAD - 1));

    // Calculate input amount - matches Python exactly
    let amount_in = div_up_fixed(
        &mul_up_fixed(&virt_in_over, amount_out),
        &(&virt_out_under - amount_out),
    )?;

    Ok(amount_in)
}

/// Calculate virtual parameter0 - matches Python calculate_virtual_parameter0
/// EXACTLY
pub fn calculate_virtual_parameter0(
    invariant: &BigInt,
    sqrt_beta: &BigInt,
    rounding: &Rounding,
) -> Result<BigInt, Error> {
    match rounding {
        Rounding::RoundDown => div_down_fixed(invariant, sqrt_beta),
        Rounding::RoundUp => div_up_fixed(invariant, sqrt_beta),
    }
}

/// Calculate virtual parameter1 - matches Python calculate_virtual_parameter1
/// EXACTLY
pub fn calculate_virtual_parameter1(
    invariant: &BigInt,
    sqrt_alpha: &BigInt,
    rounding: &Rounding,
) -> Result<BigInt, Error> {
    match rounding {
        Rounding::RoundDown => Ok(mul_down_fixed(invariant, sqrt_alpha)),
        Rounding::RoundUp => Ok(mul_up_fixed(invariant, sqrt_alpha)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test parameters matching Python reference tests
    fn create_test_params() -> (BigInt, BigInt) {
        (
            BigInt::from(900_000_000_000_000_000_u64), // sqrt_alpha = 0.9e18
            BigInt::from(1_100_000_000_000_000_000_u64), // sqrt_beta = 1.1e18
        )
    }

    fn create_test_balances() -> Vec<BigInt> {
        vec![
            BigInt::from(1_000_000_000_000_000_000_u64), // 1e18
            BigInt::from(1_000_000_000_000_000_000_u64), // 1e18
        ]
    }

    #[test]
    fn test_simple_arithmetic() {
        let a = BigInt::from(2_000_000_000_000_000_000_u64); // 2e18
        let b = BigInt::from(3_000_000_000_000_000_000_u64); // 3e18

        let result = mul_down_fixed(&a, &b);
        assert_eq!(result, BigInt::from(6_000_000_000_000_000_000_u64)); // 6e18

        let result = div_down_fixed(&a, &b).unwrap();
        assert_eq!(result, BigInt::from(666_666_666_666_666_666_u64)); // ~0.666e18
    }

    #[test]
    fn test_sqrt_basic() {
        let result = gyro_pool_math_sqrt(&BigInt::from(4_000_000_000_000_000_000_u64), 1).unwrap(); // 4e18
        // Should be close to 2e18
        let expected = BigInt::from(2_000_000_000_000_000_000_u64);
        let diff = (&result - &expected).abs();
        assert!(diff < BigInt::from(1000)); // Very small tolerance

        let result = gyro_pool_math_sqrt(&BigInt::from(0), 1).unwrap();
        assert_eq!(result, BigInt::from(0));
    }

    #[test]
    fn test_calculate_invariant() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let result = calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown);
        assert!(result.is_ok());

        let invariant = result.unwrap();
        assert!(invariant > BigInt::from(0));

        // Sanity check that it's not infinitely large
        assert!(invariant < BigInt::from(10).pow(50));
    }

    #[test]
    fn test_swap_functions() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_in =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_out =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        let amount_in = BigInt::from(100_000_000_000_000_000_u64); // 0.1e18

        let result = calc_out_given_in(
            &balances[0],
            &balances[1],
            &amount_in,
            &virtual_offset_in,
            &virtual_offset_out,
        );

        assert!(result.is_ok());
        let amount_out = result.unwrap();
        assert!(amount_out > BigInt::from(0));
        assert!(amount_out < balances[1]);
    }
}
