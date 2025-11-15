pub use shared::sources::balancer_v2::pool_fetching::Gyro3CLPPool as Pool;
use {
    crate::domain::liquidity,
    ethereum_types::{H160, H256, U256},
    shared::sources::balancer_v2::{
        pool_fetching::{CommonPoolState, Gyro3CLPPoolVersion, TokenState},
        swap::fixed_point::Bfp,
    },
};

/// Converts a domain pool into a [`shared`] Balancer V2 Gyroscope 3-CLP pool.
/// Returns `None` if the domain pool cannot be represented as a boundary pool.
pub fn to_boundary_pool(address: H160, pool: &liquidity::gyro_3clp::Pool) -> Option<Pool> {
    // NOTE: this is only used for encoding and not for solving, so it's OK to
    // use this an approximate value for now. In fact, Balancer V2 pool IDs
    // are `pool address || pool kind || pool index`, so this approximation is
    // pretty good.
    let id = {
        let mut buf = [0_u8; 32];
        buf[..20].copy_from_slice(address.as_bytes());
        H256(buf)
    };

    let swap_fee = to_fixed_point(&pool.fee)?;
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
        common: CommonPoolState {
            id,
            address,
            swap_fee,
            paused: false,
        },
        reserves,
        version: match pool.version {
            liquidity::gyro_3clp::Version::V1 => Gyro3CLPPoolVersion::V1,
        },
        // Convert Gyro 3-CLP static parameter from FixedPoint to Bfp
        root3_alpha: to_fixed_point(&pool.root3_alpha)?,
    })
}

/// Converts a rational to a Balancer fixed point number.
fn to_fixed_point(ratio: &crate::domain::eth::Rational) -> Option<Bfp> {
    // Balancer "fixed point numbers" are in a weird decimal FP format (instead
    // of a base 2 FP format you typically see). Just convert our ratio into
    // this format.
    let base = ethereum_types::U256::exp10(18);
    let wei = ratio.numer().checked_mul(base)? / ratio.denom();
    Some(Bfp::from_wei(wei))
}

/// Converts a rational to a U256.
/// Note: Rate is already in wei (18 decimals), so we just convert the rational
/// directly.
fn to_u256(ratio: &crate::domain::eth::Rational) -> Option<U256> {
    ratio.numer().checked_div(*ratio.denom())
}
