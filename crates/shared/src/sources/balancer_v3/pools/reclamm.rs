//! Module implementing ReCLAMM pool specific indexing logic for Balancer V3.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v3::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::{Result, anyhow},
    contracts::{BalancerV3ReClammPool, BalancerV3ReClammPoolFactoryV2},
    ethcontract::{BlockId, H160, U256},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        if pool.pool_type_enum() != PoolType::ReClamm {
            return Err(anyhow!(
                "Expected RECLAMM pool type, got {:?}",
                pool.pool_type_enum()
            ));
        }
        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::ReClamm, pool, block_created)?,
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
    pub version: Version,
    // ReCLAMM dynamic fields used by swap math
    pub last_virtual_balances: Vec<U256>,
    pub daily_price_shift_base: Bfp,
    pub last_timestamp: u64,
    pub centeredness_margin: Bfp,
    pub start_fourth_root_price_ratio: Bfp,
    pub end_fourth_root_price_ratio: Bfp,
    pub price_ratio_update_start_time: u64,
    pub price_ratio_update_end_time: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V2, // BalancerV3ReClammPoolFactoryV2
}

// Re-export for external use, to match other pool modules
pub type TokenState = common::TokenState;

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3ReClammPoolFactoryV2 {
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
        let pool_contract =
            BalancerV3ReClammPool::at(&self.raw_instance().web3(), pool_info.common.address);

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_dynamic = pool_contract
            .get_re_clamm_pool_dynamic_data()
            .block(block)
            .call();

        async move {
            // Join the shared common state and pool-specific dynamic data
            let (common, dynamic) = futures::try_join!(fetch_common, fetch_dynamic)?;

            // dynamic is a tuple following ReClammPoolDynamicData ABI
            let (
                _balances_live_scaled18,
                _token_rates,
                _static_swap_fee_percentage,
                _total_supply,
                last_timestamp_u256,
                last_virtual_balances,
                _daily_price_shift_exponent,
                daily_price_shift_base_u256,
                centeredness_margin_u256,
                _current_price_ratio,
                _current_fourth_root_price_ratio,
                start_fourth_root_price_ratio_u256,
                end_fourth_root_price_ratio_u256,
                price_ratio_update_start_time_u32,
                price_ratio_update_end_time_u32,
                _is_pool_initialized,
                _is_pool_paused,
                _is_pool_in_recovery_mode,
            ) = dynamic;

            let pool_state = PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                version: Version::V2,
                last_virtual_balances,
                daily_price_shift_base: Bfp::from_wei(daily_price_shift_base_u256),
                last_timestamp: last_timestamp_u256.low_u64(),
                centeredness_margin: Bfp::from_wei(centeredness_margin_u256),
                start_fourth_root_price_ratio: Bfp::from_wei(start_fourth_root_price_ratio_u256),
                end_fourth_root_price_ratio: Bfp::from_wei(end_fourth_root_price_ratio_u256),
                price_ratio_update_start_time: price_ratio_update_start_time_u32 as u64,
                price_ratio_update_end_time: price_ratio_update_end_time_u32 as u64,
            };

            Ok(Some(pool_state))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::graph_api::{DynamicData, GqlChain, PoolData, Token},
        ethcontract::H160,
    };

    #[test]
    fn convert_graph_pool_to_reclamm_pool_info() {
        let pool = PoolData {
            id: format!("0x{}", hex::encode(H160([1; 20]).0)),
            address: H160([1; 20]),
            pool_type: "RECLAMM".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x11; 20]),
                    decimals: 18,
                    weight: None,
                    price_rate_provider: None,
                },
                Token {
                    address: H160([0x22; 20]),
                    decimals: 6,
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
                    id: H160([1; 20]),
                    address: H160([1; 20]),
                    tokens: vec![H160([0x11; 20]), H160([0x22; 20])],
                    scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(12)],
                    rate_providers: vec![H160::zero(), H160::zero()],
                    block_created: 42,
                },
            },
        );
    }
}
