//! QuantAMM mathematical utilities following balancer-maths implementation
//! exactly. This module provides QuantAMM-specific swap calculations that
//! follow the services pattern with pre-computed interpolated weights.

use {
    super::{error::Error, weighted_math},
    crate::sources::balancer_v3::swap::fixed_point::Bfp,
    ethcontract::{I256, U256},
};

/// QuantAMM swap calculation: out given exact in.
/// Follows services pattern - takes pre-computed interpolated weights.
/// Uses "compute_" prefix like ReClamm for complex pools with pre-computation.
pub fn compute_out_given_in(
    balance_in: Bfp,
    weight_in: Bfp,
    balance_out: Bfp,
    weight_out: Bfp,
    amount_in: Bfp,
) -> Result<Bfp, Error> {
    // Use standard weighted pool math (matches both balancer-maths and services)
    weighted_math::calc_out_given_in(balance_in, weight_in, balance_out, weight_out, amount_in)
}

/// QuantAMM swap calculation: in given exact out.
/// Follows services pattern - takes pre-computed interpolated weights.
/// Uses "compute_" prefix like ReClamm for complex pools with pre-computation.
pub fn compute_in_given_out(
    balance_in: Bfp,
    weight_in: Bfp,
    balance_out: Bfp,
    weight_out: Bfp,
    amount_out: Bfp,
) -> Result<Bfp, Error> {
    // Use standard weighted pool math (matches both balancer-maths and services)
    weighted_math::calc_in_given_out(balance_in, weight_in, balance_out, weight_out, amount_out)
}

/// Calculate interpolated weights for a specific token pair.
/// This exactly mirrors _getNormalizedWeightPair from balancer-maths.
pub fn calculate_normalized_weight_pair(
    in_index: usize,
    out_index: usize,
    weights: &[I256],
    multipliers: &[I256],
    last_update_time: u64,
    last_interop_time: u64,
    current_timestamp: u64,
) -> Result<(Bfp, Bfp), Error> {
    if in_index >= weights.len()
        || out_index >= weights.len()
        || in_index >= multipliers.len()
        || out_index >= multipliers.len()
    {
        return Err(Error::InvalidToken);
    }

    // Determine time for multiplier calculation (matches balancer-maths exactly)
    let multiplier_time = if current_timestamp >= last_interop_time {
        last_interop_time
    } else {
        current_timestamp
    };

    let time_since_last_update = multiplier_time.saturating_sub(last_update_time);

    // Calculate weights based on time interpolation (matches balancer-maths)
    let token_in_weight = calculate_block_normalized_weight(
        weights[in_index],
        multipliers[in_index],
        time_since_last_update,
    )
    .map_err(|_| Error::InvalidToken)?;

    let token_out_weight = calculate_block_normalized_weight(
        weights[out_index],
        multipliers[out_index],
        time_since_last_update,
    )
    .map_err(|_| Error::InvalidToken)?;

    Ok((
        Bfp::from_wei(token_in_weight),
        Bfp::from_wei(token_out_weight),
    ))
}

/// Calculate interpolated weight based on time and multiplier.
/// This exactly mirrors calculateBlockNormalisedWeight from balancer-maths.
fn calculate_block_normalized_weight(
    weight: I256,
    multiplier: I256,
    time_since_last_update: u64,
) -> anyhow::Result<U256> {
    // Convert to Bfp for proper fixed-point arithmetic
    let weight_bfp = if weight >= I256::zero() {
        Bfp::from_wei(weight.into_raw())
    } else {
        return Err(anyhow::anyhow!("Negative weight not supported"));
    };

    // Critical: Scale multiplier by 1e18 EXACTLY like balancer-maths
    // const multiplierScaled18 = multiplier * BigInt('1000000000000000000');
    let multiplier_abs = multiplier.abs();
    let multiplier_scaled18 = Bfp::from_wei(
        multiplier_abs
            .into_raw()
            .checked_mul(U256::exp10(18))
            .ok_or_else(|| anyhow::anyhow!("Multiplier scaling overflow"))?,
    );

    let time_bfp = Bfp::from_wei(U256::from(time_since_last_update));

    if multiplier > I256::zero() {
        // weight + MathSol.mulDownFixed(multiplierScaled18, timeSinceLastUpdate)
        let adjustment = multiplier_scaled18
            .mul_down(time_bfp)
            .map_err(|_| anyhow::anyhow!("Weight calculation overflow"))?;
        let result = weight_bfp
            .add(adjustment)
            .map_err(|_| anyhow::anyhow!("Weight calculation overflow"))?;
        Ok(result.as_uint256())
    } else if multiplier < I256::zero() {
        // weight - MathSol.mulDownFixed(-multiplierScaled18, timeSinceLastUpdate)
        // In balancer-maths, negative multiplier uses -multiplierScaled18 (which
        // becomes positive)
        let adjustment = multiplier_scaled18
            .mul_down(time_bfp)
            .map_err(|_| anyhow::anyhow!("Weight calculation overflow"))?;
        let result = weight_bfp
            .sub(adjustment)
            .map_err(|_| anyhow::anyhow!("Weight calculation underflow"))?;
        Ok(result.as_uint256())
    } else {
        // Zero multiplier: weight remains unchanged
        Ok(weight_bfp.as_uint256())
    }
}
