use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v2::pool_fetching::Gyro3CLPPoolVersion,
    solver::liquidity::{Gyro3CLPPoolOrder, Settleable},
};

/// Median gas used per BalancerSwapGivenOutInteraction.
const GAS_PER_SWAP: u64 = 115_000;

pub fn to_domain(id: liquidity::Id, pool: Gyro3CLPPoolOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV2Gyro3CLP(balancer::v2::gyro_3clp::Pool {
            vault: vault(&pool),
            id: pool_id(&pool),
            reserves: balancer::v2::gyro_3clp::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v2::gyro_3clp::Reserve {
                            asset: eth::Asset {
                                token: token.into(),
                                amount: reserve.balance.into(),
                            },
                            scale: balancer::v2::ScalingFactor::from_raw(
                                reserve.scaling_factor.as_uint256(),
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?,
            )?,
            fee: balancer::v2::Fee::from_raw(pool.fee.as_uint256()),
            version: match pool.version {
                Gyro3CLPPoolVersion::V1 => balancer::v2::gyro_3clp::Version::V1,
            },
            // Convert Gyroscope 3-CLP static parameter from Bfp to FixedPoint
            root3_alpha: balancer::v2::gyro_3clp::FixedPoint::from_raw(
                pool.root3_alpha.as_uint256(),
            ),
        }),
    })
}

pub fn to_interaction(
    pool: &balancer::v2::gyro_3clp::Pool,
    input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> eth::Interaction {
    let vault_pool = super::Pool {
        vault: pool.vault,
        id: pool.id,
    };
    super::to_interaction(&vault_pool, input, output, receiver)
}

fn vault(pool: &Gyro3CLPPoolOrder) -> eth::ContractAddress {
    pool.settlement_handling()
        .as_any()
        .downcast_ref::<solver::liquidity::balancer_v2::SettlementHandler>()
        .expect("Gyro3CLPPoolOrder created by BalancerV2 should have BalancerV2 settlement handler")
        .vault()
        .address()
        .into()
}

fn pool_id(pool: &Gyro3CLPPoolOrder) -> balancer::v2::Id {
    pool.settlement_handling()
        .as_any()
        .downcast_ref::<solver::liquidity::balancer_v2::SettlementHandler>()
        .expect("Gyro3CLPPoolOrder created by BalancerV2 should have BalancerV2 settlement handler")
        .pool_id()
        .into()
}
