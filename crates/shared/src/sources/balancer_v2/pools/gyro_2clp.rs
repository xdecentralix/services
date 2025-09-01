//! Module implementing Gyroscope 2-CLP pool specific indexing logic.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v2::{
        graph_api::{PoolData, PoolType},
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
    anyhow::{Result, anyhow},
    contracts::{BalancerV2Gyro2CLPPool, BalancerV2Gyro2CLPPoolFactory},
    ethcontract::{BlockId, H160},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    pub sqrt_alpha: SBfp,
    pub sqrt_beta: SBfp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, common::TokenState>,
    pub swap_fee: Bfp,
    pub version: Version,
    // Gyro 2-CLP static parameters (included in PoolState for easier access)
    pub sqrt_alpha: SBfp,
    pub sqrt_beta: SBfp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        let sqrt_alpha = pool
            .sqrt_alpha
            .ok_or_else(|| anyhow!("missing sqrt_alpha for pool {:?}", pool.id))?;
        let sqrt_beta = pool
            .sqrt_beta
            .ok_or_else(|| anyhow!("missing sqrt_beta for pool {:?}", pool.id))?;

        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::Gyro2CLP, pool, block_created)?,
            sqrt_alpha,
            sqrt_beta,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV2Gyro2CLPPoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        // For Gyroscope 2-CLP, we need to fetch the immutable parameters from the pool
        // contract
        let pool_contract = BalancerV2Gyro2CLPPool::at(&self.raw_instance().web3(), pool.address);

        let sqrt_params = pool_contract.get_sqrt_parameters().call().await?;

        Ok(PoolInfo {
            common: pool,
            sqrt_alpha: SBfp::from_wei(
                sqrt_params[0]
                    .try_into()
                    .map_err(|_| anyhow!("sqrt_alpha value too large for I256"))?,
            ),
            sqrt_beta: SBfp::from_wei(
                sqrt_params[1]
                    .try_into()
                    .map_err(|_| anyhow!("sqrt_beta value too large for I256"))?,
            ),
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
            sqrt_alpha: pool_info.sqrt_alpha,
            sqrt_beta: pool_info.sqrt_beta,
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
    fn convert_graph_pool_to_gyro_2clp_pool_info() {
        let pool = PoolData {
            id: "0x1234567890123456789012345678901234567890123456789012345678901234".to_string(),
            address: H160::from_low_u64_be(1),
            pool_type: "GYRO".to_string(),
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
            sqrt_alpha: Some(SBfp::from_wei(I256::from(900_000_000_000_000_000i64))), /* sqrt_alpha = 0.9e18 */
            sqrt_beta: Some(SBfp::from_wei(I256::from(1_100_000_000_000_000_000i64))), /* sqrt_beta = 1.1e18 */
            root3_alpha: None,
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 12345).unwrap();
        assert_eq!(
            pool_info.sqrt_alpha,
            SBfp::from_wei(I256::from(900_000_000_000_000_000i64))
        );
        assert_eq!(
            pool_info.sqrt_beta,
            SBfp::from_wei(I256::from(1_100_000_000_000_000_000i64))
        );
    }
}
