pub use shared::sources::balancer_v3::pool_fetching::Gyro2CLPPool as Pool;
use {
    crate::domain::{eth, liquidity},
    ethereum_types::{H160, U256},
    shared::sources::balancer_v3::{
        pool_fetching::{CommonPoolState, Gyro2CLPPoolVersion, TokenState},
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
};

/// Converts a domain pool into a [`shared`] Balancer V3 Gyroscope 2-CLP pool.
/// Returns `None` if the domain pool cannot be represented as a boundary pool.
pub fn to_boundary_pool(address: H160, pool: &liquidity::gyro_2clp::Pool) -> Option<Pool> {
    // NOTE: this is only used for encoding and not for solving, so it's OK to
    // use this an approximate value for now. In fact, Balancer V3 pool IDs
    // are the pool addresses themselves.
    let id = address;

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
            liquidity::gyro_2clp::Version::V1 => Gyro2CLPPoolVersion::V1,
        },
        // Convert Gyro 2-CLP static parameters from Rational to SBfp
        sqrt_alpha: to_signed_fixed_point(&pool.sqrt_alpha)?,
        sqrt_beta: to_signed_fixed_point(&pool.sqrt_beta)?,
    })
}

/// Converts a rational to a Balancer fixed point number.
fn to_fixed_point(ratio: &eth::Rational) -> Option<Bfp> {
    // Balancer "fixed point numbers" are in a weird decimal FP format (instead
    // of a base 2 FP format you typically see). Just convert our ratio into
    // this format.
    let base = U256::exp10(18);
    let wei = ratio.numer().checked_mul(base)? / ratio.denom();
    Some(Bfp::from_wei(wei))
}

/// Converts a signed rational to a Balancer signed fixed point number.
fn to_signed_fixed_point(ratio: &eth::SignedRational) -> Option<SBfp> {
    // For SignedRational (based on I256), we can work directly with signed values
    let base = ethcontract::I256::exp10(18);
    let scaled = ratio.numer().checked_mul(base)? / *ratio.denom();
    Some(SBfp::from_wei(scaled))
}

/// Converts a rational to a U256.
/// Note: Rate is already in wei (18 decimals), so we just convert the rational directly.
fn to_u256(ratio: &eth::Rational) -> Option<U256> {
    ratio.numer().checked_div(*ratio.denom())
}
