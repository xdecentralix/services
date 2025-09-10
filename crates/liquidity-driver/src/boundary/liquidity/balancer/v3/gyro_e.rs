use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::GyroEPoolVersion,
    solver::liquidity::{BalancerV3GyroEOrder, balancer_v3},
};

/// Median gas used per BalancerSwapGivenOutInteraction.
// estimated with https://dune.com/queries/639857
const GAS_PER_SWAP: u64 = 88_892;

pub fn to_domain(id: liquidity::Id, pool: BalancerV3GyroEOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3GyroE(balancer::v3::gyro_e::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::gyro_e::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::gyro_e::Reserve {
                            asset: eth::Asset {
                                token: token.into(),
                                amount: reserve.balance.into(),
                            },
                            scale: balancer::v3::ScalingFactor::from_raw(
                                reserve.scaling_factor.as_uint256(),
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?,
            )?,
            fee: balancer::v3::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                GyroEPoolVersion::V1 => balancer::v3::gyro_e::Version::V1,
            },
            // Convert Gyroscope E-CLP static parameters from SBfp to SignedFixedPoint
            params_alpha: balancer::v3::gyro_e::SignedFixedPoint::from_raw(
                pool.params_alpha.as_i256(),
            ),
            params_beta: balancer::v3::gyro_e::SignedFixedPoint::from_raw(
                pool.params_beta.as_i256(),
            ),
            params_c: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.params_c.as_i256()),
            params_s: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.params_s.as_i256()),
            params_lambda: balancer::v3::gyro_e::SignedFixedPoint::from_raw(
                pool.params_lambda.as_i256(),
            ),
            tau_alpha_x: balancer::v3::gyro_e::SignedFixedPoint::from_raw(
                pool.tau_alpha_x.as_i256(),
            ),
            tau_alpha_y: balancer::v3::gyro_e::SignedFixedPoint::from_raw(
                pool.tau_alpha_y.as_i256(),
            ),
            tau_beta_x: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.tau_beta_x.as_i256()),
            tau_beta_y: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.tau_beta_y.as_i256()),
            u: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.u.as_i256()),
            v: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.v.as_i256()),
            w: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.w.as_i256()),
            z: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.z.as_i256()),
            d_sq: balancer::v3::gyro_e::SignedFixedPoint::from_raw(pool.d_sq.as_i256()),
        }),
    })
}

fn batch_router(pool: &BalancerV3GyroEOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3GyroEOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::gyro_e::Pool,
    input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> eth::Interaction {
    super::to_interaction(
        &super::Pool {
            batch_router: pool.batch_router,
            id: pool.id,
        },
        input,
        output,
        receiver,
    )
}
