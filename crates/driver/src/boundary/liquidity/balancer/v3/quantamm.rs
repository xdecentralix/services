use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v3::pool_fetching::QuantAmmPoolVersion,
    solver::liquidity::{BalancerV3QuantAmmOrder, balancer_v3},
};

/// Median gas used per BalancerV3SwapGivenOutInteraction.
// QuantAMM pools have higher gas cost due to weight calculations
const GAS_PER_SWAP: u64 = 180_000;

pub fn to_domain(id: liquidity::Id, pool: BalancerV3QuantAmmOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV3QuantAmm(balancer::v3::quantamm::Pool {
            batch_router: batch_router(&pool),
            id: pool_id(&pool),
            reserves: balancer::v3::quantamm::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v3::quantamm::Reserve {
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
                QuantAmmPoolVersion::V1 => balancer::v3::quantamm::Version::V1,
            },
            max_trade_size_ratio: balancer::v3::ScalingFactor::from_raw(
                pool.max_trade_size_ratio.as_uint256(),
            )?,
            first_four_weights_and_multipliers: pool.first_four_weights_and_multipliers,
            second_four_weights_and_multipliers: pool.second_four_weights_and_multipliers,
            last_update_time: pool.last_update_time,
            last_interop_time: pool.last_interop_time,
            current_timestamp: pool.current_timestamp,
        }),
    })
}

fn batch_router(pool: &BalancerV3QuantAmmOrder) -> eth::ContractAddress {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .batch_router()
        .address()
        .into()
}

fn pool_id(pool: &BalancerV3QuantAmmOrder) -> balancer::v3::Id {
    pool.settlement_handling
        .as_any()
        .downcast_ref::<balancer_v3::SettlementHandler>()
        .expect("downcast balancer v3 settlement handler")
        .pool_id()
        .into()
}

pub fn to_interaction(
    pool: &balancer::v3::quantamm::Pool,
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
