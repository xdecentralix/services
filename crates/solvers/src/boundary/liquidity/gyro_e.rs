pub use shared::sources::balancer_v2::pool_fetching::GyroEPool as Pool;
use {
    crate::domain::{eth, liquidity},
    ethereum_types::{H160, H256, U256},
    shared::sources::balancer_v2::{
        pool_fetching::{CommonPoolState, GyroEPoolVersion, TokenState},
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
};

/// Converts a domain pool into a [`shared`] Balancer V2 Gyroscope E-CLP pool. Returns
/// `None` if the domain pool cannot be represented as a boundary pool.
pub fn to_boundary_pool(address: H160, pool: &liquidity::gyro_e::Pool) -> Option<Pool> {
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
                    rate: U256::exp10(18),
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
            liquidity::gyro_e::Version::V1 => GyroEPoolVersion::V1,
        },
        // Convert all Gyro E-CLP static parameters from Rational to SBfp
        params_alpha: to_signed_fixed_point(&pool.params_alpha)?,
        params_beta: to_signed_fixed_point(&pool.params_beta)?,
        params_c: to_signed_fixed_point(&pool.params_c)?,
        params_s: to_signed_fixed_point(&pool.params_s)?,
        params_lambda: to_signed_fixed_point(&pool.params_lambda)?,
        tau_alpha_x: to_signed_fixed_point(&pool.tau_alpha_x)?,
        tau_alpha_y: to_signed_fixed_point(&pool.tau_alpha_y)?,
        tau_beta_x: to_signed_fixed_point(&pool.tau_beta_x)?,
        tau_beta_y: to_signed_fixed_point(&pool.tau_beta_y)?,
        u: to_signed_fixed_point(&pool.u)?,
        v: to_signed_fixed_point(&pool.v)?,
        w: to_signed_fixed_point(&pool.w)?,
        z: to_signed_fixed_point(&pool.z)?,
        d_sq: to_signed_fixed_point(&pool.d_sq)?,
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
    let base = ethcontract::I256::from(10u64.pow(18));
    
    // Convert I256 to ethcontract::I256 for calculation
    let numer_str = ratio.numer().to_string();
    let denom_str = ratio.denom().to_string();
    
    let numer_i256 = ethcontract::I256::from_dec_str(&numer_str).ok()?;
    let denom_i256 = ethcontract::I256::from_dec_str(&denom_str).ok()?;
    
    // Calculate wei value: (numer * base) / denom
    let wei_i256 = numer_i256.checked_mul(base)?.checked_div(denom_i256)?;
    
    Some(SBfp::from_wei(wei_i256))
}