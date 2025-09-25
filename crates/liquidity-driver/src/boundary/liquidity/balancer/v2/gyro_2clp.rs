use {
    crate::{
        boundary::Result,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
    },
    shared::sources::balancer_v2::pool_fetching::Gyro2CLPPoolVersion,
    solver::liquidity::{Gyro2CLPPoolOrder, Settleable},
};

/// Median gas used per BalancerSwapGivenOutInteraction.
// estimated with https://dune.com/queries/639857
const GAS_PER_SWAP: u64 = 88_892;

pub fn to_domain(id: liquidity::Id, pool: Gyro2CLPPoolOrder) -> Result<liquidity::Liquidity> {
    Ok(liquidity::Liquidity {
        id,
        gas: GAS_PER_SWAP.into(),
        kind: liquidity::Kind::BalancerV2Gyro2CLP(balancer::v2::gyro_2clp::Pool {
            vault: vault(&pool),
            id: pool_id(&pool),
            reserves: balancer::v2::gyro_2clp::Reserves::try_new(
                pool.reserves
                    .into_iter()
                    .map(|(token, reserve)| {
                        Ok(balancer::v2::gyro_2clp::Reserve {
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
                Gyro2CLPPoolVersion::V1 => balancer::v2::gyro_2clp::Version::V1,
            },
            // Convert Gyroscope 2-CLP static parameters from SBfp to SignedFixedPoint
            sqrt_alpha: balancer::v2::gyro_2clp::SignedFixedPoint::from_raw(
                pool.sqrt_alpha.as_i256(),
            ),
            sqrt_beta: balancer::v2::gyro_2clp::SignedFixedPoint::from_raw(
                pool.sqrt_beta.as_i256(),
            ),
        }),
    })
}

pub fn to_interaction(
    pool: &balancer::v2::gyro_2clp::Pool,
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

fn vault(pool: &Gyro2CLPPoolOrder) -> eth::ContractAddress {
    pool.settlement_handling()
        .as_any()
        .downcast_ref::<solver::liquidity::balancer_v2::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .vault()
        .address()
        .into()
}

fn pool_id(pool: &Gyro2CLPPoolOrder) -> balancer::v2::Id {
    pool.settlement_handling()
        .as_any()
        .downcast_ref::<solver::liquidity::balancer_v2::SettlementHandler>()
        .expect("downcast balancer settlement handler")
        .pool_id()
        .into()
}
