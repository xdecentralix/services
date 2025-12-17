//! Module implementing stable pool specific indexing logic for Balancer V3.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::{
        conversions::U256Ext as _,
        sources::balancer_v3::{
            graph_api::{PoolData, PoolType},
            swap::fixed_point::Bfp,
        },
    },
    anyhow::{Result, ensure},
    contracts::alloy::{
        BalancerV3StablePool,
        BalancerV3StablePoolFactory,
        BalancerV3StablePoolFactoryV2,
    },
    ethcontract::{BlockId, H160, U256},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    futures::{FutureExt as _, future::BoxFuture},
    num::BigRational,
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        if pool.pool_type != "STABLE" {
            return Err(anyhow::anyhow!(
                "Expected STABLE pool type, got {}",
                pool.pool_type
            ));
        }
        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::Stable, pool, block_created)?,
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
    pub amplification_parameter: AmplificationParameter,
    pub version: Version,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmplificationParameter {
    factor: U256,
    precision: U256,
}

impl AmplificationParameter {
    pub fn try_new(factor: U256, precision: U256) -> Result<Self> {
        ensure!(!precision.is_zero(), "Zero precision not allowed");
        Ok(Self { factor, precision })
    }

    /// This is the format used to pass into smart contracts.
    pub fn with_base(&self, base: U256) -> Option<U256> {
        Some(self.factor.checked_mul(base)? / self.precision)
    }

    /// This is the format used to pass along to HTTP solver.
    pub fn as_big_rational(&self) -> BigRational {
        // We can assert that the precision is non-zero as we check when constructing
        // new `AmplificationParameter` instances that this invariant holds, and we
        // don't allow modifications of `self.precision` such that it could
        // become 0.
        debug_assert!(!self.precision.is_zero());
        BigRational::new(self.factor.to_big_int(), self.precision.to_big_int())
    }

    pub fn factor(&self) -> U256 {
        self.factor
    }

    pub fn precision(&self) -> U256 {
        self.precision
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1, // BalancerV3StablePoolFactory
    V2, // BalancerV3StablePoolFactoryV2
}

// Re-export for external use
pub type TokenState = common::TokenState;

#[cfg(test)]
fn pool_state(
    version: Version,
    _info: PoolInfo,
    common: BoxFuture<'static, common::PoolState>,
    amplification_parameter: AmplificationParameter,
) -> BoxFuture<'static, Result<Option<PoolState>>> {
    async move {
        let common = common.await;
        let tokens = common.tokens;
        let swap_fee = common.swap_fee;

        Ok(Some(PoolState {
            tokens,
            swap_fee,
            amplification_parameter,
            version,
        }))
    }
    .boxed()
}

// FactoryIndexing implementation for BalancerV3StablePoolFactory (V1)
#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3StablePoolFactory::Instance {
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
        let block = block.into_alloy();
        let pool_contract = BalancerV3StablePool::Instance::new(
            pool_info.common.address.into_alloy(),
            self.provider().clone(),
        );

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_amplification_parameter = async move {
            pool_contract
                .getAmplificationParameter()
                .block(block)
                .call()
                .await
        };

        async move {
            let (common, amplification_parameter) =
                futures::try_join!(fetch_common, fetch_amplification_parameter)?;
            let amplification_parameter = AmplificationParameter::try_new(
                amplification_parameter.value.into_legacy(),
                amplification_parameter.precision.into_legacy(),
            )?;

            Ok(Some(PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                amplification_parameter,
                version: Version::V1,
            }))
        }
        .boxed()
    }
}

// FactoryIndexing implementation for BalancerV3StablePoolFactoryV2 (V2)
#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3StablePoolFactoryV2::Instance {
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
        let block = block.into_alloy();
        let pool_contract = BalancerV3StablePool::Instance::new(
            pool_info.common.address.into_alloy(),
            self.provider().clone(),
        );

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_amplification_parameter = async move {
            pool_contract
                .getAmplificationParameter()
                .block(block)
                .call()
                .await
        };

        async move {
            let (common, amplification_parameter) =
                futures::try_join!(fetch_common, fetch_amplification_parameter)?;
            let amplification_parameter = AmplificationParameter::try_new(
                amplification_parameter.value.into_legacy(),
                amplification_parameter.precision.into_legacy(),
            )?;

            Ok(Some(PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                amplification_parameter,
                version: Version::V2,
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::graph_api::{DynamicData, GqlChain, PoolData, Token},
        ethcontract::{H160, U256},
        futures::future,
        maplit::btreemap,
    };

    #[test]
    fn convert_graph_pool_to_stable_pool_info() {
        let pool = PoolData {
            id: format!("0x{}", const_hex::encode(H160([1; 20]).0)),
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
            hook: None,
        };

        assert_eq!(
            PoolInfo::from_graph_data(&pool, 42).unwrap(),
            PoolInfo {
                common: common::PoolInfo {
                    id: H160([1; 20]), // V3 uses H160 pool addresses
                    address: H160([1; 20]),
                    tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                    scaling_factors: vec![Bfp::exp10(17), Bfp::exp10(16)],
                    rate_providers: vec![H160::zero(), H160::zero()],
                    block_created: 42,
                },
            },
        );
    }

    #[test]
    fn errors_when_converting_wrong_pool_type() {
        let pool = PoolData {
            id: format!("0x{}", const_hex::encode(H160([1; 20]).0)),
            address: H160([1; 20]),
            pool_type: "WEIGHTED".to_string(),
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
            hook: None,
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }

    #[test]
    fn amplification_parameter_conversions() {
        assert_eq!(
            AmplificationParameter::try_new(2.into(), 3.into())
                .unwrap()
                .with_base(1000.into())
                .unwrap(),
            666.into()
        );
        assert_eq!(
            AmplificationParameter::try_new(7.into(), 8.into())
                .unwrap()
                .as_big_rational(),
            BigRational::new(7.into(), 8.into())
        );

        assert_eq!(
            AmplificationParameter::try_new(1.into(), 0.into())
                .unwrap_err()
                .to_string(),
            "Zero precision not allowed"
        );
    }

    #[test]
    fn version_enum_default() {
        assert_eq!(Version::default(), Version::V1);
    }

    #[tokio::test]
    async fn fetch_pool_state() {
        let pool_info = PoolInfo {
            common: common::PoolInfo {
                id: H160([1; 20]),
                address: H160([1; 20]),
                tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                rate_providers: vec![H160::zero(), H160::zero()],
                block_created: 42,
            },
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

        let amplification_parameter =
            AmplificationParameter::try_new(100.into(), 1.into()).unwrap();

        let pool_state = pool_state(
            Version::V1,
            pool_info.clone(),
            future::ready(common_pool_state).boxed(),
            amplification_parameter,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(pool_state.tokens.len(), 2);
        assert_eq!(pool_state.swap_fee, Bfp::from_wei(3000u64.into()));
        assert_eq!(pool_state.version, Version::V1);
        assert_eq!(pool_state.amplification_parameter.factor(), 100.into());
        assert_eq!(pool_state.amplification_parameter.precision(), 1.into());
    }
}
