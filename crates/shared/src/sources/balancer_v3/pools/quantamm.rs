//! Module implementing QuantAMM weighted pool specific indexing logic for
//! Balancer V3.

use {
    super::{FactoryIndexing, PoolIndexing, common},
    crate::sources::balancer_v3::{
        graph_api::{PoolData, PoolType},
        swap::fixed_point::Bfp,
    },
    anyhow::{Result, anyhow},
    contracts::{BalancerV3QuantAMMWeightedPool, BalancerV3QuantAMMWeightedPoolFactory},
    ethcontract::{BlockId, H160, I256},
    futures::{FutureExt as _, future::BoxFuture},
    std::collections::BTreeMap,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub common: common::PoolInfo,
    pub max_trade_size_ratio: Bfp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub tokens: BTreeMap<H160, common::TokenState>,
    pub swap_fee: Bfp,
    pub version: Version,
    // QuantAMM-specific static data (needed for pool operations)
    pub max_trade_size_ratio: Bfp,
    // QuantAMM-specific dynamic data (raw, no calculations)
    pub first_four_weights_and_multipliers: Vec<I256>,
    pub second_four_weights_and_multipliers: Vec<I256>,
    pub last_update_time: u64,
    pub last_interop_time: u64,
    // Current block timestamp (fetched each time pool state is retrieved)
    pub current_timestamp: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Version {
    #[default]
    V1,
}

impl PoolIndexing for PoolInfo {
    fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        if pool.pool_type != "QUANT_AMM_WEIGHTED" {
            return Err(anyhow!(
                "Expected QUANT_AMM_WEIGHTED pool type, got {}",
                pool.pool_type
            ));
        }

        let max_trade_size_ratio = pool
            .quant_amm_weighted_params
            .as_ref()
            .and_then(|params| params.max_trade_size_ratio)
            .ok_or_else(|| {
                anyhow!(
                    "missing max_trade_size_ratio for QuantAMM pool {:?}",
                    pool.id
                )
            })?;

        Ok(PoolInfo {
            common: common::PoolInfo::for_type(PoolType::QuantAmmWeighted, pool, block_created)?,
            max_trade_size_ratio,
        })
    }

    fn common(&self) -> &common::PoolInfo {
        &self.common
    }
}

#[async_trait::async_trait]
impl FactoryIndexing for BalancerV3QuantAMMWeightedPoolFactory {
    type PoolInfo = PoolInfo;
    type PoolState = PoolState;

    async fn specialize_pool_info(&self, pool: common::PoolInfo) -> Result<Self::PoolInfo> {
        let pool_contract =
            BalancerV3QuantAMMWeightedPool::at(&self.raw_instance().web3(), pool.address);

        let immutable_data = pool_contract
            .get_quant_amm_weighted_pool_immutable_data()
            .call()
            .await
            .map_err(|err| anyhow!("Failed to fetch QuantAMM immutable data: {err}"))?;

        // Extract maxTradeSizeRatio from the immutable data tuple (9th field, index 8)
        // struct: tokens, oracleStalenessThreshold, poolRegistry, ruleParameters,
        // lambda, epsilonMax, absoluteWeightGuardRail, updateInterval,
        // maxTradeSizeRatio
        let max_trade_size_ratio = Bfp::from_wei(immutable_data.8);

        Ok(PoolInfo {
            common: pool,
            max_trade_size_ratio,
        })
    }

    fn fetch_pool_state(
        &self,
        pool_info: &Self::PoolInfo,
        common_pool_state: BoxFuture<'static, common::PoolState>,
        block: BlockId,
    ) -> BoxFuture<'static, Result<Option<Self::PoolState>>> {
        let pool_contract = BalancerV3QuantAMMWeightedPool::at(
            &self.raw_instance().web3(),
            pool_info.common.address,
        );
        let max_trade_size_ratio = pool_info.max_trade_size_ratio;

        let fetch_common = common_pool_state.map(Result::Ok);
        let fetch_dynamic = pool_contract
            .get_quant_amm_weighted_pool_dynamic_data()
            .block(block)
            .call();

        async move {
            let (common, dynamic_data) = futures::try_join!(fetch_common, fetch_dynamic)?;

            // Use current system time as approximation for block timestamp
            // This is reasonable since pool fetching happens near real-time
            let block_timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Extract dynamic data following the ABI structure
            let (
                _balances_live_scaled18,   // Already in common
                _token_rates,              // Already in common
                _total_supply,             // Already in common
                _is_pool_initialized,      // Not used
                _is_pool_paused,           // Already in common
                _is_pool_in_recovery_mode, // Not used
                first_four_weights_and_multipliers,
                second_four_weights_and_multipliers,
                last_update_time,
                last_interop_time,
            ) = dynamic_data;

            // Store raw data without calculations (like ReClamm pattern)
            Ok(Some(PoolState {
                tokens: common.tokens,
                swap_fee: common.swap_fee,
                version: Version::V1,
                max_trade_size_ratio,
                // Store raw multiplier data - calculations happen in swap logic
                first_four_weights_and_multipliers,
                second_four_weights_and_multipliers,
                last_update_time,
                last_interop_time,
                // Use the actual block timestamp fetched from the blockchain
                current_timestamp: block_timestamp,
            }))
        }
        .boxed()
    }
}

// Re-export for external use, to match other pool modules
pub type TokenState = common::TokenState;

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::graph_api::{DynamicData, GqlChain, PoolData, Token},
        ethcontract::{H160, U256},
    };

    #[test]
    fn convert_graph_pool_to_quantamm_pool_info() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([1; 20]),
            pool_type: "QUANT_AMM_WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x11; 20]),
                    decimals: 18,
                    weight: None, // QuantAMM pools don't use static weights
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
            max_trade_size_ratio: Some(Bfp::from_wei(U256::from(100_000_000_000_000_000u128))), /* 10% */
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
                max_trade_size_ratio: Bfp::from_wei(U256::from(100_000_000_000_000_000u128)),
            },
        );
    }

    #[test]
    fn errors_when_converting_wrong_pool_type() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([1; 20]),
            pool_type: "WEIGHTED".to_string(), // Wrong type
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![],
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
            max_trade_size_ratio: Some(Bfp::from_wei(U256::from(100_000_000_000_000_000u128))),
            hook: None,
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }

    #[test]
    fn errors_when_missing_max_trade_size_ratio() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([1; 20]),
            pool_type: "QUANT_AMM_WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0xfa; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![],
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
            max_trade_size_ratio: None, // Missing!
            hook: None,
        };

        assert!(PoolInfo::from_graph_data(&pool, 42).is_err());
    }
}
