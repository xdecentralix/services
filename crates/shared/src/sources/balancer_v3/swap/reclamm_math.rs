//! ReCLAMM pool math, ported to Rust to mirror the balancer-maths
//! implementation.
//!
//! This module provides the functions used by the ReCLAMM AMM to compute
//! virtual balances and swap amounts. It mirrors the logic in
//! `balancer-maths/typescript/src/reClamm/reClammMath.ts` as closely as
//! possible using the existing `Bfp` fixed-point utilities.

use {
    super::{error::Error, fixed_point::Bfp},
    ethcontract::U256,
    num::{BigInt, Zero},
    number::conversions::{big_int_to_u256, u256_to_big_int},
};

#[derive(Clone, Copy, Debug)]
pub struct PriceRatioState {
    pub price_ratio_update_start_time: u64,
    pub price_ratio_update_end_time: u64,
    pub start_fourth_root_price_ratio: Bfp,
    pub end_fourth_root_price_ratio: Bfp,
}

/// Computes current virtual balances and whether they changed due to either
/// price ratio update or out-of-range centeredness drift.
pub fn compute_current_virtual_balances(
    current_timestamp: u64,
    balances_scaled18: &[Bfp; 2],
    last_virtual_balance_a: Bfp,
    last_virtual_balance_b: Bfp,
    daily_price_shift_base: Bfp,
    last_timestamp: u64,
    centeredness_margin: Bfp,
    price_ratio_state: PriceRatioState,
) -> Result<(Bfp, Bfp, bool), Error> {
    if last_timestamp == current_timestamp {
        return Ok((last_virtual_balance_a, last_virtual_balance_b, false));
    }

    let mut current_virtual_balance_a = last_virtual_balance_a;
    let mut current_virtual_balance_b = last_virtual_balance_b;

    let current_fourth_root_price_ratio = compute_fourth_root_price_ratio(
        current_timestamp,
        price_ratio_state.start_fourth_root_price_ratio,
        price_ratio_state.end_fourth_root_price_ratio,
        price_ratio_state.price_ratio_update_start_time,
        price_ratio_state.price_ratio_update_end_time,
    )?;

    let mut changed = false;

    // If the price ratio is updating, shrink/expand the price interval by
    // recalculating the virtual balances
    if current_timestamp > price_ratio_state.price_ratio_update_start_time
        && last_timestamp < price_ratio_state.price_ratio_update_end_time
    {
        let (virt_a, virt_b) = compute_virtual_balances_updating_price_ratio(
            current_fourth_root_price_ratio,
            balances_scaled18,
            last_virtual_balance_a,
            last_virtual_balance_b,
        )?;
        current_virtual_balance_a = virt_a;
        current_virtual_balance_b = virt_b;
        changed = true;
    }

    let (centeredness, is_pool_above_center) = compute_centeredness(
        balances_scaled18,
        current_virtual_balance_a,
        current_virtual_balance_b,
    )?;

    // If the pool is outside the target range, track the market price by moving the
    // price interval
    if centeredness < centeredness_margin {
        let (virt_a, virt_b) = compute_virtual_balances_updating_price_range(
            balances_scaled18,
            current_virtual_balance_a,
            current_virtual_balance_b,
            is_pool_above_center,
            daily_price_shift_base,
            current_timestamp,
            last_timestamp,
        )?;
        current_virtual_balance_a = virt_a;
        current_virtual_balance_b = virt_b;
        changed = true;
    }

    Ok((
        current_virtual_balance_a,
        current_virtual_balance_b,
        changed,
    ))
}

/// Compute amountOut for GivenIn swaps using current balances and virtual
/// balances.
pub fn compute_out_given_in(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
    token_in_index: usize,
    token_out_index: usize,
    amount_in_scaled18: Bfp,
) -> Result<Bfp, Error> {
    let (virt_in, virt_out) =
        select_virtuals(virtual_balance_a, virtual_balance_b, token_in_index)?;

    // Single-step mul/div to mirror TS bigint formula and avoid double rounding:
    // Ao = floor(((Bo + Vo) * Ai) / (Bi + Vi + Ai))
    let numerator = balances_scaled18[token_out_index].add(virt_out)?;
    let denominator = balances_scaled18[token_in_index]
        .add(virt_in)?
        .add(amount_in_scaled18)?;
    let amount_out = mul_div_down_raw(numerator, amount_in_scaled18, denominator)?;

    // Amount out cannot be greater than the real balance of the token in the pool.
    if amount_out > balances_scaled18[token_out_index] {
        return Err(Error::ProductOutOfBounds); // Align with TS error intent
    }
    Ok(amount_out)
}

/// Compute amountIn for GivenOut swaps using current balances and virtual
/// balances.
pub fn compute_in_given_out(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
    token_in_index: usize,
    token_out_index: usize,
    amount_out_scaled18: Bfp,
) -> Result<Bfp, Error> {
    // Amount out cannot be greater than the real balance of the token in the pool.
    if amount_out_scaled18 > balances_scaled18[token_out_index] {
        return Err(Error::ProductOutOfBounds);
    }

    let (virt_in, virt_out) =
        select_virtuals(virtual_balance_a, virtual_balance_b, token_in_index)?;

    // Single-step mul/div to mirror TS bigint formula and avoid double rounding:
    // Ai = ceil(((Bi + Vi) * Ao) / (Bo + Vo - Ao))
    let numerator = balances_scaled18[token_in_index].add(virt_in)?;
    let denominator = balances_scaled18[token_out_index]
        .add(virt_out)?
        .sub(amount_out_scaled18)?;
    mul_div_up_raw(numerator, amount_out_scaled18, denominator)
}

fn select_virtuals(va: Bfp, vb: Bfp, token_in_index: usize) -> Result<(Bfp, Bfp), Error> {
    match token_in_index {
        0 => Ok((va, vb)),
        1 => Ok((vb, va)),
        _ => Err(Error::InvalidToken),
    }
}

fn compute_virtual_balances_updating_price_ratio(
    current_fourth_root_price_ratio: Bfp,
    balances_scaled18: &[Bfp; 2],
    last_virtual_balance_a: Bfp,
    last_virtual_balance_b: Bfp,
) -> Result<(Bfp, Bfp), Error> {
    // Keep centeredness constant while adjusting to current price ratio
    let (centeredness, is_pool_above_center) = compute_centeredness(
        balances_scaled18,
        last_virtual_balance_a,
        last_virtual_balance_b,
    )?;

    let sqrt_price_ratio =
        current_fourth_root_price_ratio.mul_down(current_fourth_root_price_ratio)?;

    // Determine which token is undervalued (rarer)
    let (balance_undervalued, _balance_overvalued, last_undervalued, last_overvalued) =
        if is_pool_above_center {
            (
                balances_scaled18[0],
                balances_scaled18[1],
                last_virtual_balance_a,
                last_virtual_balance_b,
            )
        } else {
            (
                balances_scaled18[1],
                balances_scaled18[0],
                last_virtual_balance_b,
                last_virtual_balance_a,
            )
        };

    // Vu = Ru(1 + C + sqrt(1 + C(C + 4Q0 - 2))) / 2(Q0 - 1)
    // Compute the sqrt argument at 36-dec precision to match TS exactly.
    let one_wad_u = Bfp::one().as_uint256();
    let c_wei: BigInt = u256_to_big_int(&centeredness.as_uint256());
    let q0_wei: BigInt = u256_to_big_int(&sqrt_price_ratio.as_uint256());
    let four_q0_minus_two =
        (q0_wei.clone() * BigInt::from(4)) - u256_to_big_int(&Bfp::from(2).as_uint256());
    let inner36 = &c_wei * (&c_wei + &four_q0_minus_two)
        + BigInt::from(1_000000000000000000u128) * BigInt::from(1_000000000000000000u128);
    // The above BigInt literal equals RAY (1e36).
    let inner36_u256 = big_int_to_u256(&inner36).map_err(|_| Error::MulOverflow)?;
    let root = Bfp::from_wei(sqrt_u256(inner36_u256));

    // Numerator: Ru * (WAD + C + root) -> 36-dec
    let sum_term_wei: BigInt = u256_to_big_int(&one_wad_u)
        + u256_to_big_int(&centeredness.as_uint256())
        + u256_to_big_int(&root.as_uint256());
    let ru_wei: BigInt = u256_to_big_int(&balance_undervalued.as_uint256());
    let num36 = ru_wei * sum_term_wei;

    // Denominator: 2 * (Q0 - 1)
    let den18_wei_big: BigInt = BigInt::from(2) * (q0_wei - u256_to_big_int(&one_wad_u));
    let vu =
        Bfp::from_wei(big_int_to_u256(&(num36 / den18_wei_big)).map_err(|_| Error::MulOverflow)?);

    // vo = (vu * last_overvalued) / last_undervalued, with single-step 36->18
    // division
    let vo = mul_div_down_raw(vu, last_overvalued, last_undervalued)?;

    if is_pool_above_center {
        Ok((vu, vo))
    } else {
        Ok((vo, vu))
    }
}

fn compute_virtual_balances_updating_price_range(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
    is_pool_above_center: bool,
    daily_price_shift_base: Bfp,
    current_timestamp: u64,
    last_timestamp: u64,
) -> Result<(Bfp, Bfp), Error> {
    let sqrt_price_ratio = sqrt_36_to_18(compute_price_ratio(
        balances_scaled18,
        virtual_balance_a,
        virtual_balance_b,
    )?)?;

    let (mut v_overvalued, b_undervalued, b_overvalued) = if is_pool_above_center {
        (
            virtual_balance_b,
            balances_scaled18[0],
            balances_scaled18[1],
        )
    } else {
        (
            virtual_balance_a,
            balances_scaled18[1],
            balances_scaled18[0],
        )
    };

    // Vb = Vb * (dailyPriceShiftBase)^(T_curr - T_last)
    let exponent =
        Bfp::from_wei((U256::from(current_timestamp - last_timestamp)) * U256::exp10(18));
    let pow = pow_down_fixed(daily_price_shift_base, exponent)?;
    v_overvalued = v_overvalued.mul_down(pow)?;

    // Va = (Ra * (Vb + Rb)) / (((sqrtPriceRatio - 1) * Vb) - Rb)
    // Compute numerator in 36-dec and divide by 18-dec denominator in a single step
    // to match TS.
    let ra_wei: BigInt = u256_to_big_int(&b_undervalued.as_uint256());
    let vb_plus_rb_wei: BigInt = u256_to_big_int(&v_overvalued.add(b_overvalued)?.as_uint256());
    let va_num36 = ra_wei * vb_plus_rb_wei;
    let va_den = sqrt_price_ratio
        .sub(Bfp::one())?
        .mul_down(v_overvalued)?
        .sub(b_overvalued)?;
    let v_undervalued = Bfp::from_wei(
        big_int_to_u256(&(va_num36 / u256_to_big_int(&va_den.as_uint256())))
            .map_err(|_| Error::MulOverflow)?,
    );

    if is_pool_above_center {
        Ok((v_undervalued, v_overvalued))
    } else {
        Ok((v_overvalued, v_undervalued))
    }
}

fn compute_price_ratio(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
) -> Result<Bfp, Error> {
    let (min_price, max_price) =
        compute_price_range(balances_scaled18, virtual_balance_a, virtual_balance_b)?;
    max_price.div_up(min_price)
}

fn compute_price_range(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
) -> Result<(Bfp, Bfp), Error> {
    let invariant = compute_invariant(balances_scaled18, virtual_balance_a, virtual_balance_b)?;
    // minPrice = Vb^2 / invariant (single-step integer division to match TS/Py)
    let vb_big: BigInt = u256_to_big_int(&virtual_balance_b.as_uint256());
    let inv_big: BigInt = u256_to_big_int(&invariant);
    let vbsq_big = &vb_big * &vb_big; // 36-dec
    let min_price_big = vbsq_big / inv_big; // 18-dec result
    let min_price = Bfp::from_wei(big_int_to_u256(&min_price_big).map_err(|_| Error::MulOverflow)?);
    // maxPrice = invariant / Va^2
    let vasq = virtual_balance_a.mul_down(virtual_balance_a)?;
    let max_price = Bfp::from_wei(invariant).div_down(vasq)?;
    Ok((min_price, max_price))
}

fn compute_fourth_root_price_ratio(
    current_time: u64,
    start_fourth_root_price_ratio: Bfp,
    end_fourth_root_price_ratio: Bfp,
    price_ratio_update_start_time: u64,
    price_ratio_update_end_time: u64,
) -> Result<Bfp, Error> {
    if current_time >= price_ratio_update_end_time {
        return Ok(end_fourth_root_price_ratio);
    } else if current_time <= price_ratio_update_start_time {
        return Ok(start_fourth_root_price_ratio);
    }

    let elapsed = U256::from(current_time - price_ratio_update_start_time);
    let duration = U256::from(price_ratio_update_end_time - price_ratio_update_start_time);
    if duration.is_zero() {
        return Err(Error::ZeroDivision);
    }
    let numerator = elapsed
        .checked_mul(U256::exp10(18))
        .ok_or(Error::MulOverflow)?;
    let exponent = Bfp::from_wei(numerator / duration);

    let ratio = end_fourth_root_price_ratio.div_down(start_fourth_root_price_ratio)?;
    let pow = pow_down_fixed(ratio, exponent)?;
    let current = start_fourth_root_price_ratio.mul_down(pow)?;

    let min = start_fourth_root_price_ratio.min(end_fourth_root_price_ratio);
    Ok(current.max(min))
}

fn compute_centeredness(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
) -> Result<(Bfp, bool), Error> {
    if balances_scaled18[0].is_zero() {
        return Ok((Bfp::zero(), false));
    } else if balances_scaled18[1].is_zero() {
        return Ok((Bfp::zero(), true));
    }

    let numerator = balances_scaled18[0].mul_down(virtual_balance_b)?;
    let denominator = virtual_balance_a.mul_down(balances_scaled18[1])?;
    if numerator <= denominator {
        Ok((numerator.div_down(denominator)?, false))
    } else {
        Ok((denominator.div_down(numerator)?, true))
    }
}

fn compute_invariant(
    balances_scaled18: &[Bfp; 2],
    virtual_balance_a: Bfp,
    virtual_balance_b: Bfp,
) -> Result<U256, Error> {
    let a = balances_scaled18[0].add(virtual_balance_a)?;
    let b = balances_scaled18[1].add(virtual_balance_b)?;
    let prod = a.mul_down(b)?;
    Ok(prod.as_uint256())
}

// Integer sqrt for U256 using Newton's method.
fn sqrt_u256(n: U256) -> U256 {
    if n <= U256::one() {
        return n;
    }
    // Initial approximation based on bit length
    let mut xn = U256::one();
    let mut aa = n;
    let mut shift = |bits: u32, step: u32| {
        if aa >= (U256::one() << bits) {
            aa >>= bits;
            xn <<= step;
        }
    };
    shift(128, 64);
    shift(64, 32);
    shift(32, 16);
    shift(16, 8);
    shift(8, 4);
    shift(4, 2);
    if aa >= (U256::one() << 2) {
        xn <<= 1;
    }
    xn = (xn * 3) >> 1;
    // Newton iterations
    for _ in 0..6 {
        xn = (xn + n / xn) >> 1;
    }
    let candidate = n / xn;
    if xn > candidate { xn - U256::one() } else { xn }
}

// sqrt for a 36-decimal (1e36) fixed-point, returning a 18-decimal (1e18)
// result.
fn sqrt_36_to_18(x: Bfp) -> Result<Bfp, Error> {
    let x36 = x
        .as_uint256()
        .checked_mul(U256::exp10(18))
        .ok_or(Error::MulOverflow)?;
    Ok(Bfp::from_wei(sqrt_u256(x36)))
}

// Compute base^exp with rounding down (FixedPoint), mirroring
// MathSol.powDownFixed.
fn pow_down_fixed(base: Bfp, exp: Bfp) -> Result<Bfp, Error> {
    base.pow_down_v3(exp)
}

// Helpers to perform a single-step mul/div on raw 18-decimal fixed-point
// integers, matching the TS reference implementation's bigint behavior exactly.
fn mul_div_down_raw(a: Bfp, b: Bfp, c: Bfp) -> Result<Bfp, Error> {
    if c.is_zero() {
        return Err(Error::ZeroDivision);
    }
    let a_big: BigInt = u256_to_big_int(&a.as_uint256());
    let b_big: BigInt = u256_to_big_int(&b.as_uint256());
    let c_big: BigInt = u256_to_big_int(&c.as_uint256());
    let prod = a_big * b_big;
    let q = prod / c_big; // floor division
    let q_u256 = big_int_to_u256(&q).map_err(|_| Error::MulOverflow)?;
    Ok(Bfp::from_wei(q_u256))
}

fn mul_div_up_raw(a: Bfp, b: Bfp, c: Bfp) -> Result<Bfp, Error> {
    if c.is_zero() {
        return Err(Error::ZeroDivision);
    }
    let a_big: BigInt = u256_to_big_int(&a.as_uint256());
    let b_big: BigInt = u256_to_big_int(&b.as_uint256());
    let c_big: BigInt = u256_to_big_int(&c.as_uint256());
    let prod = a_big * b_big;
    let (q, r) = (prod.clone() / &c_big, prod % &c_big);
    let q_up = if r.is_zero() { q } else { q + BigInt::from(1) };
    let q_u256 = big_int_to_u256(&q_up).map_err(|_| Error::MulOverflow)?;
    Ok(Bfp::from_wei(q_u256))
}
