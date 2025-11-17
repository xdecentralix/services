use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    ethrpc::alloy::conversions::IntoLegacy,
    shared::sources::balancer_v2::pool_fetching::GyroEPoolVersion,
    solver::liquidity::{GyroEPoolOrder, balancer_v2},
};

/// Median gas used per BalancerSwapGivenOutInteraction.
// estimated with https://dune.com/queries/639857
const GAS_PER_SWAP: u64 = 88_892;

pub fn to_domain(id: liquidity::Id, pool: GyroEPoolOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV2GyroE(balancer::v2::gyro_e::Pool {
            vault: vault(&pool),
            id: pool_id(&pool),
            reserves: balancer::v2::gyro_e::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v2::gyro_e::Reserve {
                            asset: eth::Asset {
                                token: token.into(),
                                amount: reserve.balance.into(),
                            },
                            scale: balancer::v2::ScalingFactor::from_raw(
                                reserve.scaling_factor.as_uint256(),
                            )?,
                            rate: reserve.rate.into(),
                        })
                    })
                    .collect::<Result<_>>()?,
            )?,
            fee: balancer::v2::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                GyroEPoolVersion::V1 => balancer::v2::gyro_e::Version::V1,
            },
            // Convert Gyroscope E-CLP static parameters from SBfp to SignedFixedPoint
            params_alpha: balancer::v2::gyro_e::SignedFixedPoint::from_raw(
                pool.params_alpha.as_i256(),
            ),
            params_beta: balancer::v2::gyro_e::SignedFixedPoint::from_raw(
                pool.params_beta.as_i256(),
            ),
            params_c: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.params_c.as_i256()),
            params_s: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.params_s.as_i256()),
            params_lambda: balancer::v2::gyro_e::SignedFixedPoint::from_raw(
                pool.params_lambda.as_i256(),
            ),
            tau_alpha_x: balancer::v2::gyro_e::SignedFixedPoint::from_raw(
                pool.tau_alpha_x.as_i256(),
            ),
            tau_alpha_y: balancer::v2::gyro_e::SignedFixedPoint::from_raw(
                pool.tau_alpha_y.as_i256(),
            ),
            tau_beta_x: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.tau_beta_x.as_i256()),
            tau_beta_y: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.tau_beta_y.as_i256()),
            u: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.u.as_i256()),
            v: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.v.as_i256()),
            w: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.w.as_i256()),
            z: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.z.as_i256()),
            d_sq: balancer::v2::gyro_e::SignedFixedPoint::from_raw(pool.d_sq.as_i256()),
        }),
    })
}

fn vault(pool: &GyroEPoolOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v2::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .vault()
        .into_legacy()
        .into()
}

fn pool_id(pool: &GyroEPoolOrder) -> balancer::v2::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v2::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v2::gyro_e::Pool,
    input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> eth::Interaction {
    super::to_interaction(
        &super::Pool {
            vault: pool.vault,
            id: pool.id,
        },
        input,
        output,
        receiver,
    )
}
