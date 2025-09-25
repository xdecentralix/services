//! Module implementing StableSurge pool specific indexing logic for Balancer
//! V3. StableSurge pools are stable pools with a dynamic fee hook that
//! implements surge pricing based on imbalance.

use {
    super::{FactoryIndexing, PoolIndexing, common, stable},
    crate::sources::balancer_v3::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::Result,
    contracts::{
        BalancerV3StablePool,
        BalancerV3StableSurgeHook,
        BalancerV3StableSurgePoolFactory,
        BalancerV3StableSurgePoolFactoryV2,
    },
    ethcontract::{BlockId, H160},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    // StableSurge-specific permanent parameters (extracted from hook config)
    pub surge_threshold_percentage: Bfp,
    pub max_surge_fee_percentage: Bfp,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        // StableSurge pools come through the API as "STABLE" pools with StableSurge
        // hooks We separate them based on hook presence, not pool_type
        if pool.pool_type != "STABLE" {
            return Err(anyhow::anyhow!(
                "Expected STABLE pool type for StableSurge (pools with StableSurge hooks), got {}",
                pool.pool_type
            ));
        }

        // Extract StableSurge hook parameters
        let hook = pool
            .hook
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("StableSurge pool must have hook configuration"))?;

        let params = hook
            .params
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("StableSurge hook must have parameters"))?;

        let surge_threshold_percentage = params.surge_threshold_percentage.ok_or_else(|| {
            anyhow::anyhow!("StableSurge hook must have surge_threshold_percentage")
        })?;

        let max_surge_fee_percentage = params.max_surge_fee_percentage.ok_or_else(|| {
            anyhow::anyhow!("StableSurge hook must have max_surge_fee_percentage")
        })?;

        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::Stable, pool, block_created)?,
            surge_threshold_percentage,
            max_surge_fee_percentage,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, common::TokenState>,
    pub swap_fee: Bfp,
    pub amplification_parameter: stable::AmplificationParameter,
    pub version: stable::Version,
    // StableSurge hook parameters (from PoolInfo)
    pub surge_threshold_percentage: Bfp,
    pub max_surge_fee_percentage: Bfp,
}

// Re-export for external use
pub type TokenState = common::TokenState;
pub use stable::{AmplificationParameter, Version};

// FactoryIndexing implementation for BalancerV3StableSurgePoolFactory (V1)
#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3StableSurgePoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        // Get hook address from factory
        let hook_address = self.get_stable_surge_hook().call().await?;

        // Create hook contract instance
        let hook_contract =
            BalancerV3StableSurgeHook::at(&self.raw_instance().web3(), hook_address);

        // Fetch surge parameters from hook contract
        let surge_threshold_percentage = hook_contract
            .get_surge_threshold_percentage(pool.address)
            .call()
            .await?;
        let max_surge_fee_percentage = hook_contract
            .get_max_surge_fee_percentage(pool.address)
            .call()
            .await?;

        Ok(PoolInfo {
            common: pool,
            surge_threshold_percentage: Bfp::from_wei(surge_threshold_percentage),
            max_surge_fee_percentage: Bfp::from_wei(max_surge_fee_percentage),
        })
    }

    fn fetch_pool_state(
        &self,
        pool_info: &Self::PoolInfo,
        common_pool_state: BoxFuture<'static, common::PoolState>,
        block: BlockId,
    ) -> BoxFuture<'static, Result<Option<Self::PoolState>>> {
        let pool_contract =
            BalancerV3StablePool::at(&self.raw_instance().web3(), pool_info.common.address);

        // Extract hook parameters from pool info
        let surge_threshold_percentage = pool_info.surge_threshold_percentage;
        let max_surge_fee_percentage = pool_info.max_surge_fee_percentage;

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_amplification_parameter = pool_contract
            .get_amplification_parameter()
            .block(block)
            .call();

        async move {
            let (common, amplification_parameter) =
                futures::try_join!(fetch_common, fetch_amplification_parameter)?;
            let amplification_parameter = {
                let (factor, _, precision) = amplification_parameter;
                stable::AmplificationParameter::try_new(factor, precision)?
            };

            Ok(Some(PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                amplification_parameter,
                version: stable::Version::V1,
                surge_threshold_percentage,
                max_surge_fee_percentage,
            }))
        }
        .boxed()
    }
}

// FactoryIndexing implementation for BalancerV3StableSurgePoolFactoryV2 (V2)
#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3StableSurgePoolFactoryV2 {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        // Get hook address from factory
        let hook_address = self.get_stable_surge_hook().call().await?;

        // Create hook contract instance
        let hook_contract =
            BalancerV3StableSurgeHook::at(&self.raw_instance().web3(), hook_address);

        // Fetch surge parameters from hook contract
        let surge_threshold_percentage = hook_contract
            .get_surge_threshold_percentage(pool.address)
            .call()
            .await?;
        let max_surge_fee_percentage = hook_contract
            .get_max_surge_fee_percentage(pool.address)
            .call()
            .await?;

        Ok(PoolInfo {
            common: pool,
            surge_threshold_percentage: Bfp::from_wei(surge_threshold_percentage),
            max_surge_fee_percentage: Bfp::from_wei(max_surge_fee_percentage),
        })
    }

    fn fetch_pool_state(
        &self,
        pool_info: &Self::PoolInfo,
        common_pool_state: BoxFuture<'static, common::PoolState>,
        block: BlockId,
    ) -> BoxFuture<'static, Result<Option<Self::PoolState>>> {
        let pool_contract =
            BalancerV3StablePool::at(&self.raw_instance().web3(), pool_info.common.address);

        // Extract hook parameters from pool info
        let surge_threshold_percentage = pool_info.surge_threshold_percentage;
        let max_surge_fee_percentage = pool_info.max_surge_fee_percentage;

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_amplification_parameter = pool_contract
            .get_amplification_parameter()
            .block(block)
            .call();

        async move {
            let (common, amplification_parameter) =
                futures::try_join!(fetch_common, fetch_amplification_parameter)?;
            let amplification_parameter = {
                let (factor, _, precision) = amplification_parameter;
                stable::AmplificationParameter::try_new(factor, precision)?
            };

            Ok(Some(PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                amplification_parameter,
                version: stable::Version::V2,
                surge_threshold_percentage,
                max_surge_fee_percentage,
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::graph_api::{
            DynamicData,
            GqlChain,
            HookConfig,
            HookParams,
            PoolData,
            Token,
        },
        ethcontract::H160,
    };

    #[test]
    fn convert_graph_pool_to_stable_surge_pool_info() {
        let hook_params = HookParams {
            max_surge_fee_percentage: Some(Bfp::from_wei(
                ethcontract::U256::from_dec_str("950000000000000000").unwrap(),
            )),
            surge_threshold_percentage: Some(Bfp::from_wei(
                ethcontract::U256::from_dec_str("300000000000000000").unwrap(),
            )),
        };

        let pool = PoolData {
            id: format!("0x{}", hex::encode(H160([1; 20]).0)),
            address: H160([1; 20]),
            pool_type: "STABLE".to_string(),
            protocol_version: 3,
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
            sqrt_alpha: None,
            sqrt_beta: None,
            quant_amm_weighted_params: None,
            hook: Some(HookConfig {
                address: H160([0x44; 20]),
                params: Some(hook_params.clone()),
            }),
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 42).unwrap();

        assert_eq!(
            pool_info.common,
            common::PoolInfo {
                id: H160([1; 20]), // V3 uses H160 pool addresses
                address: H160([1; 20]),
                tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                scaling_factors: vec![Bfp::exp10(17), Bfp::exp10(16)],
                rate_providers: vec![H160::zero(), H160::zero()],
                block_created: 42,
            }
        );

        assert_eq!(
            pool_info.max_surge_fee_percentage,
            hook_params.max_surge_fee_percentage.unwrap()
        );
        assert_eq!(
            pool_info.surge_threshold_percentage,
            hook_params.surge_threshold_percentage.unwrap()
        );
    }

    #[test]
    fn stable_surge_pool_requires_hook() {
        let pool = PoolData {
            id: format!("0x{}", hex::encode(H160([1; 20]).0)),
            address: H160([1; 20]),
            pool_type: "STABLE".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![Token {
                address: H160([0x11; 20]),
                decimals: 18,
                weight: None,
                price_rate_provider: None,
            }],
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
            hook: None, // Missing hook
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }

    #[test]
    fn stable_surge_pool_requires_hook_params() {
        let pool = PoolData {
            id: format!("0x{}", hex::encode(H160([1; 20]).0)),
            address: H160([1; 20]),
            pool_type: "STABLE".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![Token {
                address: H160([0x11; 20]),
                decimals: 18,
                weight: None,
                price_rate_provider: None,
            }],
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
            hook: Some(HookConfig {
                address: H160([0x44; 20]),
                params: None, // Missing params
            }),
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }
}
