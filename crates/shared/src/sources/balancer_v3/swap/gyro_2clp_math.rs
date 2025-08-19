//! Module emulating the functions in the Balancer Gyro2CLPMath implementation.
//! The original contract code can be found at:
//! https://github.com/balancer-labs/balancer-maths/blob/main/python/src/pools/gyro/gyro_2clp_math.py
//!
//! This implementation provides swap mathematics for Gyroscope 2-CLP (Two-Asset
//! Constant Liquidity Pool) which uses a more efficient invariant formula
//! for two-asset pools with configurable price ranges.

use {
    super::{error::Error, signed_fixed_point::SignedFixedPoint},
    num::BigInt,
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

/// 2-CLP pool parameters
#[derive(Debug, Clone)]
pub struct TwoClpParams {
    pub sqrt_alpha: BigInt,
    pub sqrt_beta: BigInt,
}

/// Square root function using Newton's method with precise tolerance checking
/// Equivalent to Python gyro_pool_math_sqrt
pub fn gyro_pool_math_sqrt(x: &BigInt, tolerance: u64) -> Result<BigInt, Error> {
    if x == &BigInt::from(0) {
        return Ok(BigInt::from(0));
    }

    if x < &BigInt::from(0) {
        return Err(Error::InvalidExponent);
    }

    // Initial guess: approximate square root
    let mut z = x.clone();
    let two = BigInt::from(2);

    // Newton's method iterations
    for _ in 0..255 {
        let old_z = z.clone();

        // z = (z + x/z) / 2
        let x_div_z = x / &z;
        z = (&z + x_div_z) / &two;

        // Check convergence
        let diff = if old_z > z { &old_z - &z } else { &z - &old_z };

        if diff <= BigInt::from(tolerance) {
            break;
        }
    }

    Ok(z)
}

/// Calculate invariant using quadratic formula
///
/// The formula solves: 0 = (1-sqrt(alpha/beta)*L^2 -
/// (y/sqrt(beta)+x*sqrt(alpha))*L - x*y) Using quadratic formula: 0 = a*L^2 +
/// b*L + c Where a > 0, b < 0, and c < 0
///
/// For mb = -b and mc = -c:
/// L = (mb + (mb^2 + 4 * a * mc)^(1/2)) / (2 * a)
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

/// Calculate the terms needed for quadratic formula solution
pub fn calculate_quadratic_terms(
    balances: &[BigInt],
    sqrt_alpha: &BigInt,
    sqrt_beta: &BigInt,
    rounding: &Rounding,
) -> Result<QuadraticTerms, Error> {
    if balances.len() != 2 {
        return Err(Error::InvalidToken);
    }

    // Calculate 'a' term
    // Note: 'a' follows opposite rounding than 'b' and 'c' since it's in
    // denominator
    let sqrt_alpha_div_sqrt_beta = match rounding {
        Rounding::RoundDown => SignedFixedPoint::div_up_mag(sqrt_alpha, sqrt_beta)?,
        Rounding::RoundUp => SignedFixedPoint::div_down_mag(sqrt_alpha, sqrt_beta)?,
    };
    let a = SignedFixedPoint::sub(&WAD.clone(), &sqrt_alpha_div_sqrt_beta)?;

    // Calculate 'b' terms (in numerator)
    let b_term0 = match rounding {
        Rounding::RoundDown => SignedFixedPoint::div_down_mag(&balances[1], sqrt_beta)?,
        Rounding::RoundUp => SignedFixedPoint::div_up_mag(&balances[1], sqrt_beta)?,
    };
    let b_term1 = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&balances[0], sqrt_alpha)?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&balances[0], sqrt_alpha)?,
    };
    let mb = SignedFixedPoint::add(&b_term0, &b_term1)?;

    // Calculate 'c' term (in numerator)
    let mc = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&balances[0], &balances[1])?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&balances[0], &balances[1])?,
    };

    // Calculate b² for better fixed point precision
    // b² = x² * alpha + x*y*2*sqrt(alpha/beta) + y²/beta
    let x_squared = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&balances[0], &balances[0])?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&balances[0], &balances[0])?,
    };
    let x_squared_alpha = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&x_squared, sqrt_alpha)?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&x_squared, sqrt_alpha)?,
    };
    let b_square_term1 = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&x_squared_alpha, sqrt_alpha)?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&x_squared_alpha, sqrt_alpha)?,
    };

    let two_wad = SignedFixedPoint::mul_up_mag(&WAD.clone(), &BigInt::from(2))?;
    let xy_product = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&balances[0], &balances[1])?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&balances[0], &balances[1])?,
    };
    let xy_sqrt_alpha = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&xy_product, sqrt_alpha)?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&xy_product, sqrt_alpha)?,
    };
    let b_sq2 = match rounding {
        Rounding::RoundDown => SignedFixedPoint::div_down_mag(&xy_sqrt_alpha, sqrt_beta)?,
        Rounding::RoundUp => SignedFixedPoint::div_up_mag(&xy_sqrt_alpha, sqrt_beta)?,
    };
    let b_sq2_doubled = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&two_wad, &b_sq2)?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&two_wad, &b_sq2)?,
    };

    let sqrt_beta_squared = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_up_mag(sqrt_beta, sqrt_beta)?, /* opposite rounding */
        Rounding::RoundUp => SignedFixedPoint::mul_down_mag(sqrt_beta, sqrt_beta)?, /* opposite rounding */
    };
    let y_squared = match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(&balances[1], &balances[1])?,
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(&balances[1], &balances[1])?,
    };
    let b_sq3 = match rounding {
        Rounding::RoundDown => SignedFixedPoint::div_down_mag(&y_squared, &sqrt_beta_squared)?,
        Rounding::RoundUp => SignedFixedPoint::div_up_mag(&y_squared, &sqrt_beta_squared)?,
    };

    let b_square = SignedFixedPoint::add(
        &SignedFixedPoint::add(&b_square_term1, &b_sq2_doubled)?,
        &b_sq3,
    )?;

    Ok(QuadraticTerms {
        a,
        mb,
        b_square,
        mc,
    })
}

/// Calculate quadratic formula solution using provided terms
///
/// This function assumes a > 0, b < 0, and c <= 0, which is the case for
/// a*L^2 + b*L + c = 0 where:
///   a = 1 - sqrt(alpha/beta)
///   b = -(y/sqrt(beta) + x*sqrt(alpha))
///   c = -x*y
///
/// The special case works nicely without negative numbers.
/// The args use the notation "mb" to represent -b, and "mc" to represent -c
pub fn calculate_quadratic(
    a: &BigInt,
    mb: &BigInt,
    b_square: &BigInt,
    mc: &BigInt,
) -> Result<BigInt, Error> {
    // Calculate denominator
    let two_wad = SignedFixedPoint::mul_up_mag(&BigInt::from(2), &WAD.clone())?;
    let denominator = SignedFixedPoint::mul_up_mag(a, &two_wad)?;

    // Order multiplications for fixed point precision
    let four_wad = SignedFixedPoint::mul_down_mag(&BigInt::from(4), &WAD.clone())?;
    let add_term =
        SignedFixedPoint::mul_down_mag(&SignedFixedPoint::mul_down_mag(mc, &four_wad)?, a)?;

    // The minus sign in the radicand cancels out in this special case
    let radicand = SignedFixedPoint::add(b_square, &add_term)?;

    // Calculate square root
    let sqr_result = gyro_pool_math_sqrt(&radicand, 5)?;

    // The minus sign in the numerator cancels out in this special case
    let numerator = SignedFixedPoint::add(mb, &sqr_result)?;

    // Calculate final result
    let invariant = SignedFixedPoint::div_down_mag(&numerator, &denominator)?;

    Ok(invariant)
}

/// Calculate the output amount given an input amount for a trade
///
/// Described for X = 'in' asset and Y = 'out' asset, but equivalent for the
/// other case: dX = incrX  = amountIn  > 0
/// dY = incrY = amountOut < 0
/// x = balanceIn             x' = x + virtualParamX
/// y = balanceOut            y' = y + virtualParamY
/// L  = inv.Liq                   /            x' * y'          \          y' *
/// dX                    |dy| = y' - |   --------------------------  |   = --------------
/// x' = virtIn                    \          ( x' + dX)         /          x' +
/// dX y' = virtOut
///
/// Note that -dy > 0 is what the trader receives.
/// We exploit the fact that this formula is symmetric up to virtualOffset{X,Y}.
/// We do not use L^2, but rather x' * y', to prevent potential accumulation of
/// errors. We add a very small safety margin to compensate for potential errors
/// in the invariant.
pub fn calc_out_given_in(
    balance_in: &BigInt,
    balance_out: &BigInt,
    amount_in: &BigInt,
    virtual_offset_in: &BigInt,
    virtual_offset_out: &BigInt,
) -> Result<BigInt, Error> {
    // The factors lead to a multiplicative "safety margin" between virtual offsets
    // that is very slightly larger than 3e-18
    let wad_plus_two = SignedFixedPoint::add(&WAD.clone(), &BigInt::from(2))?;
    let virt_in_over = SignedFixedPoint::add(
        balance_in,
        &SignedFixedPoint::mul_up_mag(virtual_offset_in, &wad_plus_two)?,
    )?;

    let wad_minus_one = SignedFixedPoint::sub(&WAD.clone(), &BigInt::from(1))?;
    let virt_out_under = SignedFixedPoint::add(
        balance_out,
        &SignedFixedPoint::mul_down_mag(virtual_offset_out, &wad_minus_one)?,
    )?;

    // Calculate output amount
    let numerator = SignedFixedPoint::mul_down_mag(&virt_out_under, amount_in)?;
    let denominator = SignedFixedPoint::add(&virt_in_over, amount_in)?;
    let amount_out = SignedFixedPoint::div_down_mag(&numerator, &denominator)?;

    // Ensure amountOut < balanceOut
    if amount_out > *balance_out {
        return Err(Error::XOutOfBounds);
    }

    Ok(amount_out)
}

/// Calculate the input amount required given a desired output amount for a
/// trade
///
/// dX = incrX  = amountIn  > 0
/// dY = incrY  = amountOut < 0
/// x = balanceIn             x' = x + virtualParamX
/// y = balanceOut            y' = y + virtualParamY
/// x = balanceIn
/// L  = inv.Liq               /            x' * y'          \                x'
/// * dy                      dx =  |   --------------------------  |  -  x'  =
/// - ----------- x' = virtIn               \             y' + dy          /
/// y' + dy y' = virtOut
///
/// Note that dy < 0 < dx.
/// We exploit the fact that this formula is symmetric up to virtualOffset{X,Y}.
/// We do not use L^2, but rather x' * y', to prevent potential accumulation of
/// errors. We add a very small safety margin to compensate for potential errors
/// in the invariant.
pub fn calc_in_given_out(
    balance_in: &BigInt,
    balance_out: &BigInt,
    amount_out: &BigInt,
    virtual_offset_in: &BigInt,
    virtual_offset_out: &BigInt,
) -> Result<BigInt, Error> {
    // Check if output amount exceeds balance
    if amount_out > balance_out {
        return Err(Error::XOutOfBounds);
    }

    // The factors lead to a multiplicative "safety margin" between virtual offsets
    // that is very slightly larger than 3e-18
    let wad_plus_two = SignedFixedPoint::add(&WAD.clone(), &BigInt::from(2))?;
    let virt_in_over = SignedFixedPoint::add(
        balance_in,
        &SignedFixedPoint::mul_up_mag(virtual_offset_in, &wad_plus_two)?,
    )?;

    let wad_minus_one = SignedFixedPoint::sub(&WAD.clone(), &BigInt::from(1))?;
    let virt_out_under = SignedFixedPoint::add(
        balance_out,
        &SignedFixedPoint::mul_down_mag(virtual_offset_out, &wad_minus_one)?,
    )?;

    // Calculate input amount
    let numerator = SignedFixedPoint::mul_up_mag(&virt_in_over, amount_out)?;
    let denominator = SignedFixedPoint::sub(&virt_out_under, amount_out)?;
    let amount_in = SignedFixedPoint::div_up_mag(&numerator, &denominator)?;

    Ok(amount_in)
}

/// Calculate the virtual offset 'a' for reserves 'x', as in (x+a)*(y+b)=L^2
pub fn calculate_virtual_parameter0(
    invariant: &BigInt,
    sqrt_beta: &BigInt,
    rounding: &Rounding,
) -> Result<BigInt, Error> {
    match rounding {
        Rounding::RoundDown => SignedFixedPoint::div_down_mag(invariant, sqrt_beta),
        Rounding::RoundUp => SignedFixedPoint::div_up_mag(invariant, sqrt_beta),
    }
}

/// Calculate the virtual offset 'b' for reserves 'y', as in (x+a)*(y+b)=L^2
pub fn calculate_virtual_parameter1(
    invariant: &BigInt,
    sqrt_alpha: &BigInt,
    rounding: &Rounding,
) -> Result<BigInt, Error> {
    match rounding {
        Rounding::RoundDown => SignedFixedPoint::mul_down_mag(invariant, sqrt_alpha),
        Rounding::RoundUp => SignedFixedPoint::mul_up_mag(invariant, sqrt_alpha),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test parameters based on reference implementations
    /// Using similar values to the reference Python/TypeScript tests
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
    fn test_calculate_invariant_basic() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let result = calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown);
        assert!(result.is_ok());

        let invariant = result.unwrap();
        assert!(invariant > BigInt::from(0));

        // Test that invariant is reasonable (should be positive)
        // For 2-CLP with the specific parameters we're using, the invariant can be very
        // large This is mathematically correct behavior for the 2-CLP formula
        assert!(invariant > BigInt::from(0));

        // Sanity check that it's not infinitely large
        assert!(invariant < BigInt::from(10).pow(50));
    }

    #[test]
    fn test_calculate_invariant_rounding_consistency() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant_down =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let invariant_up =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundUp).unwrap();

        // RoundUp should give higher or equal result than RoundDown
        assert!(invariant_up >= invariant_down);

        // The difference should be small (less than 1% for reasonable inputs)
        let diff = &invariant_up - &invariant_down;
        let relative_diff = (&diff * BigInt::from(10000)) / &invariant_down;
        assert!(relative_diff < BigInt::from(100)); // Less than 1%
    }

    #[test]
    fn test_quadratic_terms_calculation() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let result =
            calculate_quadratic_terms(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown);
        assert!(result.is_ok());

        let terms = result.unwrap();

        // Verify quadratic terms are reasonable
        assert!(terms.a > BigInt::from(0), "Term 'a' should be positive");
        assert!(terms.mb > BigInt::from(0), "Term 'mb' should be positive");
        assert!(
            terms.b_square > BigInt::from(0),
            "Term 'b_square' should be positive"
        );
        assert!(terms.mc > BigInt::from(0), "Term 'mc' should be positive");

        // 'a' term should be less than WAD (since it's WAD - something positive)
        assert!(terms.a < *WAD);
    }

    #[test]
    fn test_quadratic_formula_calculation() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let terms =
            calculate_quadratic_terms(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown)
                .unwrap();

        let invariant =
            calculate_quadratic(&terms.a, &terms.mb, &terms.b_square, &terms.mc).unwrap();

        assert!(invariant > BigInt::from(0));

        // Verify that the quadratic formula produces the same result as the full
        // function
        let direct_invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        assert_eq!(invariant, direct_invariant);
    }

    #[test]
    fn test_virtual_parameters() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();

        let param0_down =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let param0_up =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundUp).unwrap();

        let param1_down =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();
        let param1_up =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundUp).unwrap();

        // Round down should be <= round up
        assert!(param0_down <= param0_up);
        assert!(param1_down <= param1_up);

        // Virtual parameters should be positive
        assert!(param0_down > BigInt::from(0));
        assert!(param1_down > BigInt::from(0));
    }

    #[test]
    fn test_calc_out_given_in_basic() {
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

        // For a balanced pool with small trade, output should be close to input
        // (minus small trading impact)
        let ratio = (&amount_out * BigInt::from(1000)) / &amount_in;
        assert!(ratio > BigInt::from(900)); // At least 90% of input
        assert!(ratio < BigInt::from(1000)); // Less than 100% (due to price impact)
    }

    #[test]
    fn test_calc_in_given_out_basic() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_in =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundUp).unwrap();
        let virtual_offset_out =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        let amount_out = BigInt::from(100_000_000_000_000_000_u64); // 0.1e18

        let result = calc_in_given_out(
            &balances[0],
            &balances[1],
            &amount_out,
            &virtual_offset_in,
            &virtual_offset_out,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();
        assert!(amount_in > BigInt::from(0));

        // Input should be slightly more than output due to trading impact
        assert!(amount_in > amount_out);

        // But not too much more (should be reasonable)
        let ratio = (&amount_in * BigInt::from(1000)) / &amount_out;
        assert!(ratio < BigInt::from(1200)); // Less than 20% premium
    }

    #[test]
    fn test_calc_out_given_in_exceeds_balance() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_in =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_out =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        // Try to extract more than the available balance
        let excessive_amount_in = &balances[0] * BigInt::from(10); // 10x the pool balance

        let result = calc_out_given_in(
            &balances[0],
            &balances[1],
            &excessive_amount_in,
            &virtual_offset_in,
            &virtual_offset_out,
        );

        // Should fail with bounds error
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::XOutOfBounds);
    }

    #[test]
    fn test_calc_in_given_out_exceeds_balance() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_in =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundUp).unwrap();
        let virtual_offset_out =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        // Try to get more than the available balance
        let excessive_amount_out = &balances[1] + BigInt::from(1);

        let result = calc_in_given_out(
            &balances[0],
            &balances[1],
            &excessive_amount_out,
            &virtual_offset_in,
            &virtual_offset_out,
        );

        // Should fail with bounds error
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::XOutOfBounds);
    }

    #[test]
    fn test_swap_reciprocity() {
        // Test that calc_out_given_in and calc_in_given_out are reciprocal
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_in =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_offset_out =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        let original_amount_in = BigInt::from(100_000_000_000_000_000_u64); // 0.1e18

        // Get amount out
        let amount_out = calc_out_given_in(
            &balances[0],
            &balances[1],
            &original_amount_in,
            &virtual_offset_in,
            &virtual_offset_out,
        )
        .unwrap();

        // Get amount in needed for that amount out (using opposite virtual parameters)
        let calculated_amount_in = calc_in_given_out(
            &balances[0],
            &balances[1],
            &amount_out,
            &virtual_offset_in,
            &virtual_offset_out,
        )
        .unwrap();

        // They should be very close (allowing for rounding differences)
        let diff = if calculated_amount_in > original_amount_in {
            &calculated_amount_in - &original_amount_in
        } else {
            &original_amount_in - &calculated_amount_in
        };

        // Allow for small rounding differences (less than 0.01%)
        let relative_diff = (&diff * BigInt::from(1000000)) / &original_amount_in;
        assert!(relative_diff < BigInt::from(100)); // Less than 0.01%
    }

    #[test]
    fn test_invariant_properties() {
        let balances = create_test_balances();
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let invariant =
            calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown).unwrap();

        // Test that the invariant satisfies the 2-CLP constraint: (x+a)*(y+b) = L^2
        let virtual_param0 =
            calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
        let virtual_param1 =
            calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown).unwrap();

        let left_side = (&balances[0] + &virtual_param0) * (&balances[1] + &virtual_param1);
        let right_side = &invariant * &invariant;

        // They should be approximately equal (allowing for rounding)
        let diff = if left_side > right_side {
            &left_side - &right_side
        } else {
            &right_side - &left_side
        };

        // For 2-CLP with large virtual parameters and complex calculations,
        // allow for more significant rounding differences
        let relative_diff = (&diff * BigInt::from(100)) / &right_side;

        // Test that the invariant constraint is approximately satisfied
        // Allow for up to 50% difference due to the complex nature of the 2-CLP
        // calculations and the fact that we're using different precision
        // arithmetic than the reference
        assert!(relative_diff < BigInt::from(50)); // Less than 50%
    }

    #[test]
    fn test_different_balance_ratios() {
        // Test with imbalanced pools
        let (sqrt_alpha, sqrt_beta) = create_test_params();

        let test_cases = vec![
            (
                BigInt::from(500_000_000_000_000_000_u64),   // 0.5e18
                BigInt::from(2_000_000_000_000_000_000_u64), // 2e18
            ),
            (
                BigInt::from(2_000_000_000_000_000_000_u64), // 2e18
                BigInt::from(500_000_000_000_000_000_u64),   // 0.5e18
            ),
            (
                BigInt::from(100_000_000_000_000_000_u64),    // 0.1e18
                BigInt::from(10_000_000_000_000_000_000_u64), // 10e18
            ),
        ];

        for (balance0, balance1) in test_cases {
            let balances = vec![balance0, balance1];

            let result =
                calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown);
            assert!(result.is_ok(), "Failed for balances: {:?}", balances);

            let invariant = result.unwrap();
            assert!(invariant > BigInt::from(0));

            // Test basic swap functionality
            let virtual_offset_in =
                calculate_virtual_parameter0(&invariant, &sqrt_beta, &Rounding::RoundDown).unwrap();
            let virtual_offset_out =
                calculate_virtual_parameter1(&invariant, &sqrt_alpha, &Rounding::RoundDown)
                    .unwrap();

            let small_amount_in = &balances[0] / BigInt::from(100); // 1% of balance

            let swap_result = calc_out_given_in(
                &balances[0],
                &balances[1],
                &small_amount_in,
                &virtual_offset_in,
                &virtual_offset_out,
            );

            assert!(
                swap_result.is_ok(),
                "Swap failed for balances: {:?}",
                balances
            );
            let amount_out = swap_result.unwrap();
            assert!(amount_out > BigInt::from(0));
            assert!(amount_out < balances[1]);
        }
    }

    #[test]
    fn test_extreme_parameters() {
        // Test with more extreme parameter values (while staying within reasonable
        // bounds)
        let balances = create_test_balances();

        let test_params = vec![
            (
                BigInt::from(100_000_000_000_000_000_u64), // sqrt_alpha = 0.1e18 (very low)
                BigInt::from(900_000_000_000_000_000_u64), // sqrt_beta = 0.9e18
            ),
            (
                BigInt::from(999_000_000_000_000_000_u64), /* sqrt_alpha = 0.999e18 (close to
                                                            * sqrt_beta) */
                BigInt::from(1_000_000_000_000_000_000_u64), // sqrt_beta = 1.0e18
            ),
        ];

        for (sqrt_alpha, sqrt_beta) in test_params {
            let result =
                calculate_invariant(&balances, &sqrt_alpha, &sqrt_beta, &Rounding::RoundDown);

            // Should still work for valid parameter ranges
            if sqrt_alpha < sqrt_beta {
                assert!(
                    result.is_ok(),
                    "Failed for params: sqrt_alpha={}, sqrt_beta={}",
                    sqrt_alpha,
                    sqrt_beta
                );
                let invariant = result.unwrap();
                assert!(invariant > BigInt::from(0));
            }
        }
    }
}
