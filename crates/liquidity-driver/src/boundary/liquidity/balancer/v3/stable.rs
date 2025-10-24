use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::StablePoolVersion,
    solver::liquidity::{BalancerV3StablePoolOrder, balancer_v3},
};

/// Median gas used per BalancerV3SwapGivenOutInteraction.
// TODO: Estimate actual gas usage for Balancer V3 stable pools
// Using same estimate as V2 for now, should be updated with actual V3
// measurements
const GAS_PER_SWAP: u64 = 88_892;

pub fn to_domain(
    id: liquidity::Id,
    pool: BalancerV3StablePoolOrder,
) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3Stable(balancer::v3::stable::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::stable::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::stable::Reserve {
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
            amplification_parameter: balancer::v3::stable::AmplificationParameter::new(
                pool.amplification_parameter.factor(),
                pool.amplification_parameter.precision(),
            )?,
            fee: balancer::v3::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                StablePoolVersion::V1 => balancer::v3::stable::Version::V1,
                StablePoolVersion::V2 => balancer::v3::stable::Version::V2,
            },
        }),
    })
}

fn batch_router(pool: &BalancerV3StablePoolOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3StablePoolOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::stable::Pool,
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
