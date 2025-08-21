//! Module implementing Gyroscope 3-CLP pool specific indexing logic.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v2::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::{Result, anyhow},
    contracts::{BalancerV2Gyro3CLPPool, BalancerV2Gyro3CLPPoolFactory},
    ethcontract::{BlockId, H160},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    pub root3_alpha: Bfp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, common::TokenState>,
    pub swap_fee: Bfp,
    pub version: Version,
    // Gyro 3-CLP static parameter (included in PoolState for easier access)
    pub root3_alpha: Bfp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        let root3_alpha = pool
            .root3_alpha
            .ok_or_else(|| anyhow!("missing root3_alpha for pool {:?}", pool.id))?;

        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::Gyro3CLP, pool, block_created)?,
            root3_alpha,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV2Gyro3CLPPoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        // For Gyroscope 3-CLP, we need to fetch the immutable parameter from the pool
        // contract
        let pool_contract = BalancerV2Gyro3CLPPool::at(&self.raw_instance().web3(), pool.address);

        let root3_alpha = pool_contract.get_root_3_alpha().call().await?;

        Ok(PoolInfo {
            common: pool,
            root3_alpha: Bfp::from_wei(root3_alpha),
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
        let tokens = common
            .tokens
            .into_iter()
            .map(|(address, common_state)| (address, common_state))
            .collect();

        Ok(Some(PoolState {
            tokens,
            swap_fee: common.swap_fee,
            version,
            // Pass through static parameter from PoolInfo
            root3_alpha: pool_info.root3_alpha,
        }))
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v2::{
            graph_api::{DynamicData, Token},
            swap::fixed_point::Bfp,
        },
        ethcontract::H160,
    };

    #[test]
    fn convert_graph_pool_to_gyro_3clp_pool_info() {
        let pool = PoolData {
            id: "0x1234567890123456789012345678901234567890123456789012345678901234".to_string(),
            address: H160::from_low_u64_be(1),
            pool_type: "GYRO3".to_string(),
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
                Token {
                    address: H160::from_low_u64_be(5),
                    decimals: 18,
                    weight: None,
                    price_rate_provider: None,
                },
            ],
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
            root3_alpha: Some(Bfp::from_wei(ethcontract::U256::from(
                800_000_000_000_000_000u64,
            ))),
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 12345).unwrap();
        assert_eq!(
            pool_info.root3_alpha,
            Bfp::from_wei(ethcontract::U256::from(800_000_000_000_000_000u64))
        );
    }
}
