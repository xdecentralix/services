pub use shared::sources::balancer_v3::pool_fetching::ReClammPool as Pool;
use {
    crate::domain::{eth, liquidity},
    ethereum_types::{H160, U256},
    shared::sources::balancer_v3::{
        pool_fetching::{CommonPoolState, TokenState},
        swap::fixed_point::Bfp,
    },
};

pub fn to_boundary_pool(address: H160, pool: &liquidity::reclamm::Pool) -> Option<Pool> {
    // Build CommonPoolState (V3 pools use address as ID)
    let common = CommonPoolState {
        id: address,
        address,
        swap_fee: to_fixed_point(&pool.fee)?,
        paused: false,
    };

    // Map reserves into TokenState map
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
        version: shared::sources::balancer_v3::pool_fetching::ReClammPoolVersion::V2,
        last_virtual_balances: pool
            .last_virtual_balances
            .iter()
            .map(|ratio| to_fixed_point(ratio).map(|b| b.as_uint256()))
            .collect::<Option<_>>()?,
        daily_price_shift_base: to_fixed_point(&pool.daily_price_shift_base)?,
        last_timestamp: pool.last_timestamp,
        centeredness_margin: to_fixed_point(&pool.centeredness_margin)?,
        start_fourth_root_price_ratio: to_fixed_point(&pool.start_fourth_root_price_ratio)?,
        end_fourth_root_price_ratio: to_fixed_point(&pool.end_fourth_root_price_ratio)?,
        price_ratio_update_start_time: pool.price_ratio_update_start_time,
        price_ratio_update_end_time: pool.price_ratio_update_end_time,
    })
}

fn to_fixed_point(ratio: &eth::Rational) -> Option<Bfp> {
    let base = U256::exp10(18);
    let wei = ratio.numer().checked_mul(base)? / ratio.denom();
    Some(Bfp::from_wei(wei))
}

/// Converts a rational to a U256.
/// Note: Rate is already in wei (18 decimals), so we just convert the rational
/// directly.
fn to_u256(ratio: &eth::Rational) -> Option<U256> {
    ratio.numer().checked_div(*ratio.denom())
}
