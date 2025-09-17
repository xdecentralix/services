//! Module implementing weighted pool specific indexing logic for Balancer V3.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v3::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::{Result, anyhow},
    contracts::{BalancerV3WeightedPool, BalancerV3WeightedPoolFactory},
    ethcontract::{BlockId, H160},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    pub weights: Vec<Bfp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, TokenState>,
    pub swap_fee: Bfp,
    pub version: Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenState {
    pub common: common::TokenState,
    pub weight: Bfp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        if pool.pool_type != "WEIGHTED" {
            return Err(anyhow!(
                "Expected WEIGHTED pool type, got {}",
                pool.pool_type
            ));
        }
        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::Weighted, pool, block_created)?,
            weights: pool
                .tokens()
                .iter()
                .map(|token| {
                    token
                        .weight
                        .ok_or_else(|| anyhow!("missing weights for pool {:?}", pool.id))
                })
                .collect::<Result<_>>()?,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3WeightedPoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        let pool_contract = BalancerV3WeightedPool::at(&self.raw_instance().web3(), pool.address);
        let weights = pool_contract
            .methods()
            .get_normalized_weights()
            .call()
            .await?
            .into_iter()
            .map(Bfp::from_wei)
            .collect();

        Ok(PoolInfo {
            common: pool,
            weights,
        })
    }

    fn fetch_pool_state(
        &self,
        pool_info: &Self::PoolInfo,
        common_pool_state: BoxFuture<'static, common::PoolState>,
        _: BlockId,
    ) -> BoxFuture<'static, Result<Option<Self::PoolState>>> {
        pool_state(Version::V1, pool_info.clone(), common_pool_state)
    }
}

fn pool_state(
    version: Version,
    info: PoolInfo,
    common: BoxFuture<'static, common::PoolState>,
) -> BoxFuture<'static, Result<Option<PoolState>>> {
    async move {
        let common = common.await;
        let tokens = common
            .tokens
            .into_iter()
            .zip(&info.weights)
            .map(|((address, common), &weight)| (address, TokenState { common, weight }))
            .collect();
        let swap_fee = common.swap_fee;

        Ok(Some(PoolState {
            tokens,
            swap_fee,
            version,
        }))
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::graph_api::{DynamicData, GqlChain, PoolData, Token},
        ethcontract::{BlockNumber, H160, U256},
        ethcontract_mock::Mock,
        futures::future,
        maplit::btreemap,
    };

    #[test]
    fn convert_graph_pool_to_weighted_pool_info() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([1; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x11; 20]),
                    decimals: 1,
                    weight: Some(bfp_v3!("1.337")),
                    price_rate_provider: None,
                },
                Token {
                    address: H160([0x22; 20]),
                    decimals: 2,
                    weight: Some(bfp_v3!("4.2")),
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
            sqrt_alpha: None,
            sqrt_beta: None,
            quant_amm_weighted_params: None,
            hook: None,
        };

        assert_eq!(
            PoolInfo::from_graph_data(&pool, 42).unwrap(),
            PoolInfo {
                common: common::PoolInfo {
                    id: H160([1; 20]),
                    address: H160([1; 20]),
                    tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                    scaling_factors: vec![Bfp::exp10(17), Bfp::exp10(16)],
                    rate_providers: vec![H160::zero(), H160::zero()],
                    block_created: 42,
                },
                weights: vec![
                    Bfp::from_wei(1_337_000_000_000_000_000u128.into()),
                    Bfp::from_wei(4_200_000_000_000_000_000u128.into()),
                ],
            },
        );
    }

    #[test]
    fn errors_when_converting_wrong_pool_type() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([1; 20]),
            pool_type: "STABLE".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x11; 20]),
                    decimals: 1,
                    weight: Some(bfp_v3!("1.337")),
                    price_rate_provider: None,
                },
                Token {
                    address: H160([0x22; 20]),
                    decimals: 2,
                    weight: Some(bfp_v3!("4.2")),
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
            sqrt_alpha: None,
            sqrt_beta: None,
            quant_amm_weighted_params: None,
            hook: None,
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }

    #[tokio::test]
    async fn fetch_weighted_pool() {
        let weights = [bfp_v3!("0.5"), bfp_v3!("0.25"), bfp_v3!("0.25")];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        pool.expect_call(BalancerV3WeightedPool::signatures().get_normalized_weights())
            .returns(weights.iter().copied().map(Bfp::as_uint256).collect());

        let factory = BalancerV3WeightedPoolFactory::at(&web3, H160([0xfa; 20]));
        let pool = factory
            .specialize_pool_info(common::PoolInfo {
                id: H160([1; 20]),
                address: pool.address(),
                tokens: vec![H160([0x11; 20]), H160([0x22; 20]), H160([0x33; 20])],
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(0)],
                rate_providers: vec![H160::zero(), H160::zero(), H160::zero()],
                block_created: 42,
            })
            .await
            .unwrap();

        assert_eq!(pool.weights, weights);
    }

    #[tokio::test]
    async fn fetch_pool_state() {
        let mock = Mock::new(42);
        let web3 = mock.web3();

        let pool_info = PoolInfo {
            common: common::PoolInfo {
                id: H160([1; 20]),
                address: H160([1; 20]),
                tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                rate_providers: vec![H160::zero(), H160::zero()],
                block_created: 42,
            },
            weights: vec![
                Bfp::from_wei(500_000_000_000_000_000u128.into()),
                Bfp::from_wei(500_000_000_000_000_000u128.into()),
            ],
        };

        let common_pool_state = common::PoolState {
            paused: false,
            swap_fee: Bfp::from_wei(3000u64.into()),
            tokens: btreemap! {
                H160([0x11; 20]) => common::TokenState {
                    balance: 1000u64.into(),
                    scaling_factor: Bfp::exp10(0),
                    rate: U256::exp10(18),
                },
                H160([0x22; 20]) => common::TokenState {
                    balance: 2000u64.into(),
                    scaling_factor: Bfp::exp10(0),
                    rate: U256::exp10(18),
                },
            },
        };

        let factory = BalancerV3WeightedPoolFactory::at(&web3, H160([0xfa; 20]));
        let pool_state = factory
            .fetch_pool_state(
                &pool_info,
                future::ready(common_pool_state).boxed(),
                BlockId::Number(BlockNumber::Latest),
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(pool_state.tokens.len(), 2);
        assert_eq!(pool_state.swap_fee, Bfp::from_wei(3000u64.into()));
        assert_eq!(pool_state.version, Version::V1);
    }
}
