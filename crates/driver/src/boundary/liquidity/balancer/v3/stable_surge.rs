use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::StablePoolVersion,
    solver::liquidity::{BalancerV3StableSurgePoolOrder, balancer_v3},
};

/// Median gas used per BalancerV3StableSurgeSwapGivenOutInteraction.
/// StableSurge pools have slightly higher gas cost due to dynamic fee
/// calculation in the hook, but the base swap is still the same as stable
/// pools.
// TODO: Estimate actual gas usage for Balancer V3 StableSurge pools
// Using a slightly higher estimate than stable pools for the hook overhead
const GAS_PER_SWAP: u64 = 93_000;

pub fn to_domain(
    id: liquidity::Id,
    pool: BalancerV3StableSurgePoolOrder,
) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3StableSurge(balancer::v3::stable_surge::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::stable_surge::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::stable_surge::Reserve {
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
            amplification_parameter: balancer::v3::stable_surge::AmplificationParameter::new(
                pool.amplification_parameter.factor(),
                pool.amplification_parameter.precision(),
            )?,
            fee: balancer::v3::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                StablePoolVersion::V1 => balancer::v3::stable_surge::Version::V1,
                StablePoolVersion::V2 => balancer::v3::stable_surge::Version::V2,
            },
            surge_threshold_percentage: balancer::v3::stable_surge::SurgeThresholdPercentage::new(
                pool.surge_threshold_percentage.as_uint256(),
            )?,
            max_surge_fee_percentage: balancer::v3::stable_surge::MaxSurgeFeePercentage::new(
                pool.max_surge_fee_percentage.as_uint256(),
            )?,
        }),
    })
}

fn batch_router(pool: &BalancerV3StableSurgePoolOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3StableSurgePoolOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::stable_surge::Pool,
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
