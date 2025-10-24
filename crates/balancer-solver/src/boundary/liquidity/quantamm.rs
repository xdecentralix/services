pub use shared::sources::balancer_v3::pool_fetching::QuantAmmPool as Pool;
use {
    crate::domain::{eth, liquidity},
    ethcontract::I256,
    ethereum_types::{H160, U256},
    shared::sources::balancer_v3::{
        pool_fetching::{CommonPoolState, QuantAmmPoolVersion, TokenState},
        swap::fixed_point::Bfp,
    },
};

pub fn to_boundary_pool(address: H160, pool: &liquidity::quantamm::Pool) -> Option<Pool> {
    // Build CommonPoolState (V3 pools use address as ID)
    let common = CommonPoolState {
        id: address,
        address,
        swap_fee: to_fixed_point(&pool.fee)?,
        paused: false,
    };

    // Map reserves into regular TokenState (same as ReClamm pattern)
    let reserves = pool
        .reserves
        .iter()
        .map(|reserve| {
            Some((
                reserve.asset.token.0,
                TokenState {
                    balance: reserve.asset.amount,
                    scaling_factor: to_fixed_point(&reserve.scale.get())?,
                    rate: to_u256(&reserve.rate)?,
                },
            ))
        })
        .collect::<Option<_>>()?;

    Some(Pool {
        common,
        reserves,
        version: QuantAmmPoolVersion::V1,
        max_trade_size_ratio: to_fixed_point(&pool.max_trade_size_ratio)?,
        first_four_weights_and_multipliers: pool
            .first_four_weights_and_multipliers
            .iter()
            .map(to_signed_i256)
            .collect::<Option<_>>()?,
        second_four_weights_and_multipliers: pool
            .second_four_weights_and_multipliers
            .iter()
            .map(to_signed_i256)
            .collect::<Option<_>>()?,
        last_update_time: pool.last_update_time,
        last_interop_time: pool.last_interop_time,
        current_timestamp: pool.current_timestamp,
    })
}

fn to_fixed_point(ratio: &eth::Rational) -> Option<Bfp> {
    let base = U256::exp10(18);
    let wei = ratio.numer().checked_mul(base)? / ratio.denom();
    Some(Bfp::from_wei(wei))
}

fn to_signed_i256(ratio: &eth::SignedRational) -> Option<I256> {
    // Follow the same pattern as GyroE's to_signed_fixed_point function
    let base = I256::from(10u64.pow(18));

    // Convert I256 to ethcontract::I256 for calculation
    let numer_str = ratio.numer().to_string();
    let denom_str = ratio.denom().to_string();

    let numer_i256 = I256::from_dec_str(&numer_str).ok()?;
    let denom_i256 = I256::from_dec_str(&denom_str).ok()?;

    // Calculate wei value: (numer * base) / denom
    let wei_i256 = numer_i256.checked_mul(base)?.checked_div(denom_i256)?;

    Some(wei_i256)
}

/// Converts a rational to a U256.
/// Note: Rate is already in wei (18 decimals), so we just convert the rational directly.
fn to_u256(ratio: &eth::Rational) -> Option<U256> {
    ratio.numer().checked_div(*ratio.denom())
}
