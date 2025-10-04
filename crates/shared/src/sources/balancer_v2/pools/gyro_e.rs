//! Module implementing Gyroscope E-CLP pool specific indexing logic.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v2::{
        graph_api::{PoolData, PoolType},
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
    anyhow::{Result, anyhow},
    contracts::alloy::{BalancerV2GyroECLPPool, BalancerV2GyroECLPPoolFactory},
    ethcontract::{BlockId, I256, H160},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    pub params_alpha: SBfp,
    pub params_beta: SBfp,
    pub params_c: SBfp,
    pub params_s: SBfp,
    pub params_lambda: SBfp,
    pub tau_alpha_x: SBfp,
    pub tau_alpha_y: SBfp,
    pub tau_beta_x: SBfp,
    pub tau_beta_y: SBfp,
    pub u: SBfp,
    pub v: SBfp,
    pub w: SBfp,
    pub z: SBfp,
    pub d_sq: SBfp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, common::TokenState>,
    pub swap_fee: Bfp,
    pub version: Version,
    // Gyro E-CLP static parameters (included in PoolState for easier access)
    pub params_alpha: SBfp,
    pub params_beta: SBfp,
    pub params_c: SBfp,
    pub params_s: SBfp,
    pub params_lambda: SBfp,
    pub tau_alpha_x: SBfp,
    pub tau_alpha_y: SBfp,
    pub tau_beta_x: SBfp,
    pub tau_beta_y: SBfp,
    pub u: SBfp,
    pub v: SBfp,
    pub w: SBfp,
    pub z: SBfp,
    pub d_sq: SBfp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        let params_alpha = pool
            .alpha
            .ok_or_else(|| anyhow!("missing alpha for pool {:?}", pool.id))?;
        let params_beta = pool
            .beta
            .ok_or_else(|| anyhow!("missing beta for pool {:?}", pool.id))?;
        let params_c = pool
            .c
            .ok_or_else(|| anyhow!("missing c for pool {:?}", pool.id))?;
        let params_s = pool
            .s
            .ok_or_else(|| anyhow!("missing s for pool {:?}", pool.id))?;
        let params_lambda = pool
            .lambda
            .ok_or_else(|| anyhow!("missing lambda for pool {:?}", pool.id))?;
        let tau_alpha_x = pool
            .tau_alpha_x
            .ok_or_else(|| anyhow!("missing tau_alpha_x for pool {:?}", pool.id))?;
        let tau_alpha_y = pool
            .tau_alpha_y
            .ok_or_else(|| anyhow!("missing tau_alpha_y for pool {:?}", pool.id))?;
        let tau_beta_x = pool
            .tau_beta_x
            .ok_or_else(|| anyhow!("missing tau_beta_x for pool {:?}", pool.id))?;
        let tau_beta_y = pool
            .tau_beta_y
            .ok_or_else(|| anyhow!("missing tau_beta_y for pool {:?}", pool.id))?;
        let u = pool
            .u
            .ok_or_else(|| anyhow!("missing u for pool {:?}", pool.id))?;
        let v = pool
            .v
            .ok_or_else(|| anyhow!("missing v for pool {:?}", pool.id))?;
        let w = pool
            .w
            .ok_or_else(|| anyhow!("missing w for pool {:?}", pool.id))?;
        let z = pool
            .z
            .ok_or_else(|| anyhow!("missing z for pool {:?}", pool.id))?;
        let d_sq = pool
            .d_sq
            .ok_or_else(|| anyhow!("missing d_sq for pool {:?}", pool.id))?;
        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::GyroE, pool, block_created)?,
            params_alpha,
            params_beta,
            params_c,
            params_s,
            params_lambda,
            tau_alpha_x,
            tau_alpha_y,
            tau_beta_x,
            tau_beta_y,
            u,
            v,
            w,
            z,
            d_sq,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV2GyroECLPPoolFactory::Instance {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        // For Gyroscope E-CLP, we need to fetch the immutable parameters from the pool
        // contract
        let pool_contract = BalancerV2GyroECLPPool::Instance::new(
            pool.address.into_alloy(),
            self.provider().clone(),
        );

        let result = pool_contract.getECLPParams().call().await?;

        // Helper to convert alloy signed I256 to ethcontract I256
        let convert_signed = |alloy_i256: alloy::primitives::I256| -> I256 {
            let underlying_u256 = alloy_i256.into_raw();
            I256::from_raw(underlying_u256.into_legacy())
        };

        Ok(PoolInfo {
            common: pool,
            params_alpha: SBfp::from_wei(convert_signed(result.params.alpha)),
            params_beta: SBfp::from_wei(convert_signed(result.params.beta)),
            params_c: SBfp::from_wei(convert_signed(result.params.c)),
            params_s: SBfp::from_wei(convert_signed(result.params.s)),
            params_lambda: SBfp::from_wei(convert_signed(result.params.lambda)),
            tau_alpha_x: SBfp::from_wei(convert_signed(result.d.tauAlpha.x)),
            tau_alpha_y: SBfp::from_wei(convert_signed(result.d.tauAlpha.y)),
            tau_beta_x: SBfp::from_wei(convert_signed(result.d.tauBeta.x)),
            tau_beta_y: SBfp::from_wei(convert_signed(result.d.tauBeta.y)),
            u: SBfp::from_wei(convert_signed(result.d.u)),
            v: SBfp::from_wei(convert_signed(result.d.v)),
            w: SBfp::from_wei(convert_signed(result.d.w)),
            z: SBfp::from_wei(convert_signed(result.d.z)),
            d_sq: SBfp::from_wei(convert_signed(result.d.dSq)),
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
    pool_info: PoolInfo,
    common: BoxFuture<'static, common::PoolState>,
) -> BoxFuture<'static, Result<Option<PoolState>>> {
    async move {
        let common = common.await;
        let tokens = common.tokens.into_iter().collect();

        Ok(Some(PoolState {
            tokens,
            swap_fee: common.swap_fee,
            version,
            // Pass through static parameters from PoolInfo
            params_alpha: pool_info.params_alpha,
            params_beta: pool_info.params_beta,
            params_c: pool_info.params_c,
            params_s: pool_info.params_s,
            params_lambda: pool_info.params_lambda,
            tau_alpha_x: pool_info.tau_alpha_x,
            tau_alpha_y: pool_info.tau_alpha_y,
            tau_beta_x: pool_info.tau_beta_x,
            tau_beta_y: pool_info.tau_beta_y,
            u: pool_info.u,
            v: pool_info.v,
            w: pool_info.w,
            z: pool_info.z,
            d_sq: pool_info.d_sq,
        }))
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v2::graph_api::{DynamicData, Token},
        ethcontract::{H160, I256},
    };

    #[test]
    fn convert_graph_pool_to_gyro_eclp_pool_info() {
        let pool = PoolData {
            id: "0x1234567890123456789012345678901234567890123456789012345678901234".to_string(),
            address: H160::from_low_u64_be(1),
            pool_type: "GYROE".to_string(),
            protocol_version: 2,
            factory: H160::from_low_u64_be(2),
            chain: crate::sources::balancer_v2::graph_api::GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160::from_low_u64_be(3),
                    decimals: 18,
                    weight: None,
                    price_rate_provider: None,
                },
                Token {
                    address: H160::from_low_u64_be(4),
                    decimals: 18,
                    weight: None,
                    price_rate_provider: None,
                },
            ],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 1234567890,
            alpha: Some(SBfp::from_wei(I256::from(1000))),
            beta: Some(SBfp::from_wei(I256::from(2000))),
            c: Some(SBfp::from_wei(I256::from(3000))),
            s: Some(SBfp::from_wei(I256::from(4000))),
            lambda: Some(SBfp::from_wei(I256::from(5000))),
            tau_alpha_x: Some(SBfp::from_wei(I256::from(6000))),
            tau_alpha_y: Some(SBfp::from_wei(I256::from(7000))),
            tau_beta_x: Some(SBfp::from_wei(I256::from(8000))),
            tau_beta_y: Some(SBfp::from_wei(I256::from(9000))),
            u: Some(SBfp::from_wei(I256::from(10000))),
            v: Some(SBfp::from_wei(I256::from(11000))),
            w: Some(SBfp::from_wei(I256::from(12000))),
            z: Some(SBfp::from_wei(I256::from(13000))),
            d_sq: Some(SBfp::from_wei(I256::from(14000))),
            sqrt_alpha: None,
            sqrt_beta: None,
            root3_alpha: None,
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 1234567890).unwrap();
        assert_eq!(pool_info.common.address, pool.address);
        assert_eq!(pool_info.params_alpha, SBfp::from_wei(I256::from(1000)));
        assert_eq!(pool_info.params_beta, SBfp::from_wei(I256::from(2000)));
        assert_eq!(pool_info.params_c, SBfp::from_wei(I256::from(3000)));
        assert_eq!(pool_info.params_s, SBfp::from_wei(I256::from(4000)));
        assert_eq!(pool_info.params_lambda, SBfp::from_wei(I256::from(5000)));
        assert_eq!(pool_info.tau_alpha_x, SBfp::from_wei(I256::from(6000)));
        assert_eq!(pool_info.tau_alpha_y, SBfp::from_wei(I256::from(7000)));
        assert_eq!(pool_info.tau_beta_x, SBfp::from_wei(I256::from(8000)));
        assert_eq!(pool_info.tau_beta_y, SBfp::from_wei(I256::from(9000)));
        assert_eq!(pool_info.u, SBfp::from_wei(I256::from(10000)));
        assert_eq!(pool_info.v, SBfp::from_wei(I256::from(11000)));
        assert_eq!(pool_info.w, SBfp::from_wei(I256::from(12000)));
        assert_eq!(pool_info.z, SBfp::from_wei(I256::from(13000)));
        assert_eq!(pool_info.d_sq, SBfp::from_wei(I256::from(14000)));
    }

    #[test]
    fn errors_when_converting_wrong_pool_type() {
        let pool = PoolData {
            id: "0x1234567890123456789012345678901234567890123456789012345678901234".to_string(),
            address: H160::from_low_u64_be(1),
            pool_type: "WEIGHTED".to_string(), // Wrong pool type
            protocol_version: 2,
            factory: H160::from_low_u64_be(2),
            chain: crate::sources::balancer_v2::graph_api::GqlChain::MAINNET,
            pool_tokens: vec![Token {
                address: H160::from_low_u64_be(3),
                decimals: 18,
                weight: None,
                price_rate_provider: None,
            }],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 1234567890,
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
            root3_alpha: None,
        };

        let result = PoolInfo::from_graph_data(&pool, 1234567890);
        assert!(result.is_err());
    }
}
