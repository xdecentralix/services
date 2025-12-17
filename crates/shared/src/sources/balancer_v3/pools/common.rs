//! Module with data types and logic common to multiple Balancer V3 pool types

use {
    super::{FactoryIndexing, Pool, PoolIndexing as _, PoolStatus},
    crate::{
        sources::balancer_v3::{
            graph_api::{PoolData, PoolType},
            swap::fixed_point::Bfp,
        },
        token_info::TokenInfoFetching,
    },
    anyhow::{Context, Result, anyhow, ensure},
    contracts::alloy::BalancerV3Vault,
    ethcontract::{BlockId, H160, U256},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    futures::{FutureExt as _, future::BoxFuture},
    std::{collections::BTreeMap, future::Future, sync::Arc},
    tokio::sync::oneshot,
};

/// Trait for fetching pool data that is generic on a factory type.
#[cfg_attr(any(test, feature = "test-util"), mockall::automock)]
#[async_trait::async_trait]
pub trait PoolInfoFetching<Factory>: Send + Sync
where
    Factory: FactoryIndexing,
{
    async fn fetch_pool_info(
        &self,
        pool_address: H160,
        block_created: u64,
    ) -> Result<Factory::PoolInfo>;

    fn fetch_pool(
        &self,
        pool: &Factory::PoolInfo,
        block: BlockId,
    ) -> BoxFuture<'static, Result<PoolStatus>>;
}

/// Generic pool info fetcher for fetching pool info and state that is generic
/// on a pool factory type and its inner pool type.
pub struct PoolInfoFetcher<Factory> {
    vault: BalancerV3Vault::Instance,
    factory: Factory,
    token_infos: Arc<dyn TokenInfoFetching>,
}

impl<Factory> PoolInfoFetcher<Factory> {
    pub fn new(
        vault: BalancerV3Vault::Instance,
        factory: Factory,
        token_infos: Arc<dyn TokenInfoFetching>,
    ) -> Self {
        Self {
            vault,
            factory,
            token_infos,
        }
    }

    /// Retrieves the scaling exponents for the specified tokens.
    async fn scaling_factors(&self, tokens: &[H160]) -> Result<Vec<Bfp>> {
        let token_infos = self.token_infos.get_token_infos(tokens).await;
        tokens
            .iter()
            .map(|token| {
                let decimals = token_infos
                    .get(token)
                    .ok_or_else(|| anyhow!("missing token info for {:?}", token))?
                    .decimals
                    .ok_or_else(|| anyhow!("missing decimals for token {:?}", token))?;
                scaling_factor_from_decimals(decimals)
            })
            .collect()
    }

    async fn fetch_common_pool_info(
        &self,
        pool_address: H160,
        block_created: u64,
    ) -> Result<PoolInfo> {
        // Get the pool ID from the pool address (for V3, pool ID is the pool address)
        let pool_id = pool_address;

        // Use V3 vault getPoolTokenInfo to get the tokens in the pool as well as the
        // rate providers
        let result = self
            .vault
            .getPoolTokenInfo(pool_address.into_alloy())
            .call()
            .await?;

        // Get the tokens from the result
        let tokens: Vec<H160> = result
            .tokens
            .into_iter()
            .map(IntoLegacy::into_legacy)
            .collect();

        // Get the rate providers from the token infos
        let rate_providers: Vec<H160> = result
            .tokenInfo
            .into_iter()
            .map(|info| info.rateProvider.into_legacy())
            .collect();

        // Get the scaling factors for the tokens through the token info fetcher
        let scaling_factors = self.scaling_factors(&tokens).await?;

        Ok(PoolInfo {
            id: pool_id,
            address: pool_address,
            tokens,
            scaling_factors,
            rate_providers,
            block_created,
        })
    }

    fn fetch_common_pool_state(
        &self,
        pool: &PoolInfo,
        block: BlockId,
    ) -> BoxFuture<'static, Result<PoolState>> {
        let block = block.into_alloy();
        let pool_address = pool.address.into_alloy();

        // Use V3 Vault isPoolPaused to get the paused status
        let vault_paused = self.vault.clone();
        let fetch_paused = async move {
            vault_paused
                .isPoolPaused(pool_address)
                .block(block)
                .call()
                .await
        };

        // Use V3 Vault getStaticSwapFeePercentage to get the swap fee
        let vault_fee = self.vault.clone();
        let fetch_swap_fee = async move {
            vault_fee
                .getStaticSwapFeePercentage(pool_address)
                .block(block)
                .call()
                .await
                .map(|r| r.into_legacy())
        };

        // Use V3 Vault getPoolData to get the pool data
        let vault_data = self.vault.clone();
        let fetch_pool_data = async move {
            vault_data
                .getPoolData(pool_address)
                .block(block)
                .call()
                .await
        };

        let vault_rates = self.vault.clone();
        let fetch_token_rates = async move {
            vault_rates
                .getPoolTokenRates(pool_address)
                .block(block)
                .call()
                .await
        };

        // Because of a `mockall` limitation, we **need** the future returned
        // here to be `'static`. This requires us to clone and move `pool` into
        // the async closure - otherwise it would only live for as long as
        // `pool`, i.e. `'_`.
        let pool = pool.clone();

        async move {
            // Get the paused status, swap fee, and pool data
            let (paused, swap_fee, pool_data, token_rates) = futures::try_join!(
                fetch_paused,
                fetch_swap_fee,
                fetch_pool_data,
                fetch_token_rates
            )?;

            // Convert the swap fee to a Bfp
            let swap_fee = Bfp::from_wei(swap_fee);

            // Pool Data: (pool_config_bits, tokens, token_infos, balances_raw,
            // balances_live_scaled18, token_rates, decimal_scaling_factors)
            let tokens: Vec<H160> = pool_data
                .tokens
                .iter()
                .map(|t| (*t).into_legacy())
                .collect();
            let balances: Vec<U256> = pool_data
                .balancesRaw
                .iter()
                .map(|b| (*b).into_legacy())
                .collect();
            let token_rates: Vec<U256> = token_rates
                .tokenRates
                .iter()
                .map(|r| (*r).into_legacy())
                .collect();

            // Ensure the number of balances matches the number of tokens
            ensure!(
                pool.tokens.len() == tokens.len(),
                "pool token mismatch: expected {} tokens, got {} tokens",
                pool.tokens.len(),
                tokens.len()
            );

            // Ensure the number of rates matches the number of tokens
            ensure!(
                pool.tokens.len() == token_rates.len(),
                "pool token rates mismatch: expected {} rates, got {} rates",
                pool.tokens.len(),
                token_rates.len()
            );

            let scaling_by_addr: std::collections::BTreeMap<ethcontract::H160, Bfp> =
                itertools::izip!(&pool.tokens, &pool.scaling_factors)
                    .map(|(&addr, &sf)| (addr, sf))
                    .collect();

            let tokens = itertools::izip!(&tokens, balances, token_rates)
                .map(|(&address, balance, rate)| {
                    let scaling_factor = *scaling_by_addr
                        .get(&address)
                        .expect("missing scaling factor for address");
                    (
                        address,
                        TokenState {
                            balance,
                            scaling_factor,
                            rate,
                        },
                    )
                })
                .collect();

            Ok(PoolState {
                paused,
                swap_fee,
                tokens,
            })
        }
        .boxed()
    }
}

#[async_trait::async_trait]
impl<Factory> PoolInfoFetching<Factory> for PoolInfoFetcher<Factory>
where
    Factory: FactoryIndexing,
{
    async fn fetch_pool_info(
        &self,
        pool_address: H160,
        block_created: u64,
    ) -> Result<Factory::PoolInfo> {
        let common_pool_info = self
            .fetch_common_pool_info(pool_address, block_created)
            .await?;
        self.factory.specialize_pool_info(common_pool_info).await
    }

    fn fetch_pool(
        &self,
        pool_info: &Factory::PoolInfo,
        block: BlockId,
    ) -> BoxFuture<'static, Result<PoolStatus>> {
        let pool_id = pool_info.common().id;
        let (common_pool_state, common_pool_state_ok) =
            share_common_pool_state(self.fetch_common_pool_state(pool_info.common(), block));
        let pool_state =
            self.factory
                .fetch_pool_state(pool_info, common_pool_state_ok.boxed(), block);

        async move {
            let common_pool_state = common_pool_state.await?;
            if common_pool_state.paused {
                return Ok(PoolStatus::Paused);
            }
            let pool_state = match pool_state.await? {
                Some(state) => state,
                None => return Ok(PoolStatus::Disabled),
            };

            Ok(PoolStatus::Active(Box::new(Pool {
                id: pool_id,
                kind: pool_state.into(),
            })))
        }
        .boxed()
    }
}

/// Common pool data shared across all Balancer V3 pools.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolInfo {
    pub id: H160, // For V3, pool ID is the pool address
    pub address: H160,
    pub tokens: Vec<H160>,
    pub scaling_factors: Vec<Bfp>,
    pub rate_providers: Vec<H160>,
    pub block_created: u64,
}

impl PoolInfo {
    /// Loads a pool info from Graph pool data.
    pub fn from_graph_data(pool: &PoolData, block_created: u64) -> Result<Self> {
        ensure!(pool.tokens().len() > 1, "insufficient tokens in pool");

        Ok(PoolInfo {
            id: pool.address, // For V3, pool ID is the pool address
            address: pool.address,
            tokens: pool.tokens().iter().map(|token| token.address).collect(),
            scaling_factors: pool
                .tokens()
                .iter()
                .map(|token| scaling_factor_from_decimals(token.decimals))
                .collect::<Result<_>>()?,
            rate_providers: pool
                .tokens()
                .iter()
                .map(|token| token.price_rate_provider.unwrap_or(H160::zero()))
                .collect(),
            block_created,
        })
    }

    /// Loads a common pool info from Graph pool data, requiring the pool type
    /// to be the specified value.
    pub fn for_type(pool_type: PoolType, pool: &PoolData, block_created: u64) -> Result<Self> {
        ensure!(
            pool.pool_type_enum() == pool_type,
            "cannot convert {:?} pool to {:?} pool",
            pool.pool_type_enum(),
            pool_type,
        );
        Self::from_graph_data(pool, block_created)
    }
}

/// Common pool state information shared across all pool types.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolState {
    pub paused: bool,
    pub swap_fee: Bfp,
    pub tokens: BTreeMap<H160, TokenState>,
}

/// Common pool token state information that is shared among all pool types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenState {
    pub balance: U256,
    pub scaling_factor: Bfp,
    pub rate: U256,
}

/// Compute the scaling rate from a Balancer pool's scaling factor.
///
/// A "scaling rate" is what the optimisation solvers (a.k.a. Quasimodo) expects
/// for token scaling, specifically, it expects a `double` that, when dividing
/// a token amount, would return its amount in base units:
///
/// ```text
///     auto in = in_unscaled / m_scaling_rates.at(t_in).convert_to<double>();
/// ```
///
/// In other words, this is the **inverse** of the scaling factor, as it is
/// defined in the Balancer V2 math.
pub fn compute_scaling_rate(scaling_factor: Bfp) -> Result<U256> {
    Bfp::exp10(18)
        .as_uint256()
        .checked_div(scaling_factor.as_uint256())
        .context("unsupported scaling factor of 0")
}

/// Converts a token decimal count to its corresponding scaling factor.
pub fn scaling_factor_from_decimals(decimals: u8) -> Result<Bfp> {
    Ok(Bfp::exp10(scaling_exponent_from_decimals(decimals)? as _))
}

/// Converts a token decimal count to its corresponding scaling exponent.
pub fn scaling_exponent_from_decimals(decimals: u8) -> Result<u8> {
    // Technically this should never fail for Balancer Pools since tokens
    // with more than 18 decimals (not supported by balancer contracts)
    // V3 uses the same scaling factor logic as V2
    18u8.checked_sub(decimals)
        .context("unsupported token with more than 18 decimals")
}

/// An internal utility method for sharing the success value for an
/// `anyhow::Result`.
///
/// Typically, this is pretty trivial using `FutureExt::shared`. However, since
/// `anyhow::Error: !Clone` we need to use a different approach.
///
/// # Panics
///
/// Polling the future with the shared success value will panic if the result
/// future has not already resolved to a `Ok` value. This method is only ever
/// meant to be used internally, so we don't have to worry that these
/// assumptions leak out of this module.
fn share_common_pool_state(
    fut: impl Future<Output = Result<PoolState>>,
) -> (
    impl Future<Output = Result<PoolState>>,
    impl Future<Output = PoolState>,
) {
    let (pool_sender, mut pool_receiver) = oneshot::channel();

    let result = fut.inspect(|pool_result| {
        let pool_result = match pool_result {
            Ok(pool) => Ok(pool.clone()),
            // We can't clone `anyhow::Error` so just use an empty `()` error.
            Err(_) => Err(()),
        };
        // Ignore error if the shared future was dropped.
        let _ = pool_sender.send(pool_result);
    });
    let shared = async move {
        pool_receiver
            .try_recv()
            .expect("result future is still pending or has been dropped")
            .expect("result future resolved to an error")
    };

    (result, shared)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            sources::balancer_v3::{
                graph_api::{DynamicData, GqlChain, PoolData, Token},
                pools::MockFactoryIndexing,
            },
            token_info::{MockTokenInfoFetching, TokenInfo},
        },
        alloy::{
            providers::{Provider, ProviderBuilder},
            sol_types::SolCall,
            transports::mock::Asserter,
        },
        contracts::alloy::BalancerV3Vault,
        ethcontract::U256,
        futures::future,
        maplit::{btreemap, hashmap},
    };

    #[tokio::test]
    async fn fetch_common_pool_info() {
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];

        let asserter = Asserter::new();
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter.clone())
            .erased();

        let pool_address = H160([0x99; 20]);
        let vault = BalancerV3Vault::Instance::new(H160::random().into_alloy(), provider.clone());

        // Mock getPoolTokenInfo response
        let get_pool_token_info_response =
            BalancerV3Vault::BalancerV3Vault::getPoolTokenInfoCall::abi_encode_returns(
                &BalancerV3Vault::BalancerV3Vault::getPoolTokenInfoReturn {
                    tokens: tokens.iter().copied().map(|t| t.into_alloy()).collect(),
                    tokenInfo: vec![
                        BalancerV3Vault::BalancerV3Vault::TokenInfo {
                            tokenType: 0,
                            rateProvider: alloy::primitives::Address::ZERO,
                            paysYieldFees: false,
                        };
                        3
                    ],
                    balancesRaw: vec![alloy::primitives::U256::ZERO; 3],
                    lastBalancesLiveScaled18: vec![alloy::primitives::U256::ZERO; 3],
                },
            );
        asserter.push_success(&get_pool_token_info_response);

        let mut token_infos = MockTokenInfoFetching::new();
        token_infos
            .expect_get_token_infos()
            .withf(move |t| t == tokens)
            .returning(move |_| {
                hashmap! {
                    tokens[0] => TokenInfo { decimals: Some(18), symbol: None },
                    tokens[1] => TokenInfo { decimals: Some(18), symbol: None },
                    tokens[2] => TokenInfo { decimals: Some(6), symbol: None },
                }
            });

        let pool_info_fetcher = PoolInfoFetcher {
            vault,
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };

        let pool_info = pool_info_fetcher
            .fetch_common_pool_info(pool_address, 1337)
            .await
            .unwrap();

        assert_eq!(
            pool_info,
            PoolInfo {
                id: pool_address,
                address: pool_address,
                tokens: tokens.to_vec(),
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(12)],
                rate_providers: vec![H160::zero(), H160::zero(), H160::zero()],
                block_created: 1337,
            }
        );
    }

    #[tokio::test]
    async fn scaling_factor_error_on_missing_info() {
        let mut token_infos = MockTokenInfoFetching::new();
        token_infos
            .expect_get_token_infos()
            .returning(|_| hashmap! {});

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::Instance::new(
                H160([0xba; 20]).into_alloy(),
                ethrpc::mock::web3().alloy,
            ),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        assert!(
            pool_info_fetcher
                .scaling_factors(&[H160([0xff; 20])])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn scaling_factor_error_on_missing_decimals() {
        let token = H160([0xff; 20]);
        let mut token_infos = MockTokenInfoFetching::new();
        token_infos.expect_get_token_infos().returning(move |_| {
            hashmap! {
                token => TokenInfo { decimals: None, symbol: None },
            }
        });

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::Instance::new(
                H160([0xba; 20]).into_alloy(),
                ethrpc::mock::web3().alloy,
            ),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        assert!(pool_info_fetcher.scaling_factors(&[token]).await.is_err());
    }

    #[test]
    fn convert_graph_pool_to_common_pool_info() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([0x22; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0x55; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![
                Token {
                    address: H160([0x33; 20]),
                    decimals: 18,
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
                    price_rate_provider: None,
                },
                Token {
                    address: H160([0x44; 20]),
                    decimals: 6,
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
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
            quant_amm_weighted_params: None,
            hook: None,
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 42).unwrap();

        assert_eq!(pool_info.id, H160([0x22; 20]));
        assert_eq!(pool_info.address, H160([0x22; 20]));
        assert_eq!(pool_info.tokens, vec![H160([0x33; 20]), H160([0x44; 20])]);
        assert_eq!(
            pool_info.scaling_factors,
            vec![Bfp::exp10(0), Bfp::exp10(12)]
        );
        assert_eq!(pool_info.block_created, 42);
    }

    #[test]
    fn pool_conversion_insufficient_tokens() {
        let pool = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([0x22; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0x55; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![Token {
                address: H160([0x33; 20]),
                decimals: 18,
                weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
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
            quant_amm_weighted_params: None,
            hook: None,
        };

        let result = PoolInfo::from_graph_data(&pool, 42);
        assert!(result.is_err());
    }

    #[test]
    fn scaling_factor_from_decimals_ok_and_err() {
        assert_eq!(scaling_factor_from_decimals(18).unwrap(), Bfp::exp10(0));
        assert_eq!(scaling_factor_from_decimals(6).unwrap(), Bfp::exp10(12));
        assert!(scaling_factor_from_decimals(19).is_err());
    }

    #[tokio::test]
    async fn share_pool_state_future() {
        let (shared_fut, shared_rx) = share_common_pool_state(future::ok(PoolState {
            paused: false,
            swap_fee: Bfp::from_wei(U256::from(3000)),
            tokens: btreemap! {},
        }));

        let result2 = shared_fut.await.unwrap();
        let result1 = shared_rx.await;

        assert_eq!(result1, result2);
    }

    #[tokio::test]
    #[should_panic]
    async fn shared_pool_state_future_panics_if_pending() {
        let (_shared_fut, shared_rx) =
            share_common_pool_state(future::pending::<Result<PoolState>>());

        let _ = shared_rx.await;
    }

    #[tokio::test]
    async fn share_pool_state_future_if_dropped() {
        let (shared_fut, _shared_rx) = share_common_pool_state(future::ok(PoolState {
            paused: false,
            swap_fee: Bfp::from_wei(U256::from(3000)),
            tokens: btreemap! {},
        }));

        // This should not panic even if the receiver is dropped
        let _ = shared_fut.await;
    }

    #[tokio::test]
    #[should_panic]
    async fn share_pool_state_future_if_errored() {
        let (shared_fut, shared_rx) =
            share_common_pool_state(future::err::<PoolState, _>(anyhow!("test error")));

        let _ = shared_fut.await;
        shared_rx.await;
    }

    #[test]
    fn compute_scaling_rates() {
        assert_eq!(
            compute_scaling_rate(scaling_factor_from_decimals(18).unwrap()).unwrap(),
            U256::from(1_000_000_000_000_000_000_u128),
        );
        assert_eq!(
            compute_scaling_rate(scaling_factor_from_decimals(6).unwrap()).unwrap(),
            U256::from(1_000_000)
        );
        assert_eq!(
            compute_scaling_rate(scaling_factor_from_decimals(0).unwrap()).unwrap(),
            U256::from(1)
        );
    }
}
