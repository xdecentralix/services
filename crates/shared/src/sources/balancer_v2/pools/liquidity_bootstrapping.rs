//! Module implementing liquidity bootstrapping pool specific indexing logic.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v2::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::Result,
    contracts::{
        BalancerV2LiquidityBootstrappingPool,
        BalancerV2LiquidityBootstrappingPoolFactory,
    },
    ethcontract::BlockId,
    futures::{FutureExt as _, future::BoxFuture},
};

pub use super::weighted::{PoolState, TokenState, Version};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        Ok(PoolInfo {
            common: common::PoolInfo::for_type(
                PoolType::LiquidityBootstrapping,
                pool,
                block_created,
            )?,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV2LiquidityBootstrappingPoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        Ok(PoolInfo { common: pool })
    }

    fn fetch_pool_state(
        &self,
        pool_info: &Self::PoolInfo,
        common_pool_state: BoxFuture<'static, common::PoolState>,
        block: BlockId,
    ) -> BoxFuture<'static, Result<Option<Self::PoolState>>> {
        let pool_contract = BalancerV2LiquidityBootstrappingPool::at(
            &self.raw_instance().web3(),
            pool_info.common.address,
        );

        let fetch_common = common_pool_state.map(Result::Ok);
        // Liquidity bootstrapping pools use dynamic weights, meaning that we
        // need to fetch them every time.
        let fetch_weights = pool_contract.get_normalized_weights().block(block).call();
        let fetch_swap_enabled = pool_contract.get_swap_enabled().block(block).call();

        async move {
            let (common, weights, swap_enabled) =
                futures::try_join!(fetch_common, fetch_weights, fetch_swap_enabled)?;
            if !swap_enabled {
                return Ok(None);
            }

            let tokens = common
                .tokens
                .into_iter()
                .zip(&weights)
                .map(|((address, common), &weight)| {
                    (
                        address,
                        TokenState {
                            common,
                            weight: Bfp::from_wei(weight),
                        },
                    )
                })
                .collect();
            let swap_fee = common.swap_fee;

            Ok(Some(PoolState {
                tokens,
                swap_fee,
                version: Version::V0,
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v2::graph_api::{DynamicData, GqlChain, PoolData, Token},
        ethcontract::{H160, H256},
    };

    #[test]
    fn errors_when_converting_wrong_pool_type() {
        let pool = PoolData {
            id: format!("0x{}", hex::encode(H256([2; 32]).0)),
            address: H160([1; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 2,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x11; 20]),
                    decimals: 1,
                    weight: None,
                    price_rate_provider: None,
                },
                Token {
                    address: H160([0x22; 20]),
                    decimals: 2,
                    weight: None,
                    price_rate_provider: None,
                },
            ],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 0,
            alpha: None,
            beta: None,
            c: None,
            s: None,
            lambda: None,
            tau_alpha_x: None,
            tau_alpha_y: None,
            tau_beta_x: None,
            tau_beta_y: None,
            u: None,
            v: None,
            w: None,
            z: None,
            d_sq: None,
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }
}
