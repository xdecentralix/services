use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    ethrpc::alloy::conversions::IntoLegacy,
    solver::liquidity::{BalancerV3ReClammOrder, balancer_v3},
};

/// Median gas used per BalancerV3SwapGivenOutInteraction.
// TODO: Estimate actual gas usage for Balancer V3
const GAS_PER_SWAP: u64 = 100_000;

pub fn to_domain(id: liquidity::Id, pool: BalancerV3ReClammOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3ReClamm(balancer::v3::reclamm::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::reclamm::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::reclamm::Reserve {
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
                shared::sources::balancer_v3::pools::reclamm::Version::V2 => {
                    balancer::v3::reclamm::Version::V2
                }
            },
            last_virtual_balances: pool.last_virtual_balances.into_iter().collect(),
            daily_price_shift_base: balancer::v3::ScalingFactor::from_raw(
                pool.daily_price_shift_base.as_uint256(),
            )?,
            last_timestamp: pool.last_timestamp,
            centeredness_margin: balancer::v3::ScalingFactor::from_raw(
                pool.centeredness_margin.as_uint256(),
            )?,
            start_fourth_root_price_ratio: balancer::v3::ScalingFactor::from_raw(
                pool.start_fourth_root_price_ratio.as_uint256(),
            )?,
            end_fourth_root_price_ratio: balancer::v3::ScalingFactor::from_raw(
                pool.end_fourth_root_price_ratio.as_uint256(),
            )?,
            price_ratio_update_start_time: pool.price_ratio_update_start_time,
            price_ratio_update_end_time: pool.price_ratio_update_end_time,
        }),
    })
}

pub fn to_interaction(
    pool: &liquidity::balancer::v3::reclamm::Pool,
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

fn batch_router(pool: &BalancerV3ReClammOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .batch_router()
        .address()
        .into_legacy()
        .into()
}

fn pool_id(pool: &BalancerV3ReClammOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .pool_id()
        .into()
}
