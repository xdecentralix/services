use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::Gyro2CLPPoolVersion,
    solver::liquidity::{BalancerV3Gyro2CLPOrder, balancer_v3},
};

/// Median gas used per BalancerSwapGivenOutInteraction.
const GAS_PER_SWAP: u64 = 88_892;

pub fn to_domain(id: liquidity::Id, pool: BalancerV3Gyro2CLPOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3Gyro2CLP(balancer::v3::gyro_2clp::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::gyro_2clp::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::gyro_2clp::Reserve {
                            asset: eth::Asset {
                                token: token.into(),
                                amount: reserve.balance.into(),
                            },
                            scale: balancer::v3::ScalingFactor::from_raw(
                                reserve.scaling_factor.as_uint256(),
                            )?,
                            rate: reserve.rate.into(),
                        })
                    })
                    .collect::<Result<_>>()?,
            )?,
            fee: balancer::v3::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                Gyro2CLPPoolVersion::V1 => balancer::v3::gyro_2clp::Version::V1,
            },
            // Convert Gyroscope 2-CLP static parameters from SBfp to SignedFixedPoint
            sqrt_alpha: balancer::v3::gyro_2clp::SignedFixedPoint::from_raw(
                pool.sqrt_alpha.as_i256(),
            ),
            sqrt_beta: balancer::v3::gyro_2clp::SignedFixedPoint::from_raw(
                pool.sqrt_beta.as_i256(),
            ),
        }),
    })
}

fn batch_router(pool: &BalancerV3Gyro2CLPOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3Gyro2CLPOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::gyro_2clp::Pool,
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
