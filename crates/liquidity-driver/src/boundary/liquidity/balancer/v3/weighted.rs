use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::WeightedPoolVersion,
    solver::liquidity::{BalancerV3WeightedProductOrder, balancer_v3},
};

/// Median gas used per BalancerV3SwapGivenOutInteraction.
// TODO: Estimate actual gas usage for Balancer V3
const GAS_PER_SWAP: u64 = 100_000;

pub fn to_domain(
    id: liquidity::Id,
    pool: BalancerV3WeightedProductOrder,
) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3Weighted(balancer::v3::weighted::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::weighted::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::weighted::Reserve {
                            asset: eth::Asset {
                                token: token.into(),
                                amount: reserve.common.balance.into(),
                            },
                            weight: balancer::v3::weighted::Weight::from_raw(
                                reserve.weight.as_uint256(),
                            ),
                            scale: balancer::v3::ScalingFactor::from_raw(
                                reserve.common.scaling_factor.as_uint256(),
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?,
            )?,
            fee: balancer::v3::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                WeightedPoolVersion::V1 => balancer::v3::weighted::Version::V1,
            },
        }),
    })
}

fn batch_router(pool: &BalancerV3WeightedProductOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3WeightedProductOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::weighted::Pool,
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
