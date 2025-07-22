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
    ethcontract::{BlockId, H160, U256},
    futures::{FutureExt as _, future::BoxFuture},
    std::{collections::BTreeMap, future::Future, sync::Arc},
    tokio::sync::oneshot,
    contracts::{BalancerV3WeightedPool, BalancerV3Vault, BalancerV3StablePool},
};

/// Trait for fetching pool data that is generic on a factory type.
#[mockall::automock]
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
    vault: BalancerV3Vault,
    factory: Factory,
    token_infos: Arc<dyn TokenInfoFetching>,
}

impl<Factory> PoolInfoFetcher<Factory> {
    pub fn new(
        vault: BalancerV3Vault,
        factory: Factory,
        token_infos: Arc<dyn TokenInfoFetching>,
    ) -> Self {
        Self {
            vault,
            factory,
            token_infos,
        }
    }
    fn weighted_pool_at(&self, pool_address: H160) -> BalancerV3WeightedPool {
        let web3 = self.vault.raw_instance().web3();
        BalancerV3WeightedPool::at(&web3, pool_address)
    }

    fn stable_pool_at(&self, pool_address: H160) -> BalancerV3StablePool {
        let web3 = self.vault.raw_instance().web3();
        BalancerV3StablePool::at(&web3, pool_address)
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
        // Create pool contract directly from address
        let pool_id = pool_address;
        let pool_contract = self.weighted_pool_at(pool_address);
        let tokens = pool_contract.methods().get_tokens().call().await?;
        let scaling_factors = self.scaling_factors(&tokens).await?;

        Ok(PoolInfo {
            id: pool_id,
            address: pool_address,
            tokens,
            scaling_factors,
            block_created,
        })
    }

    fn fetch_common_pool_state(
        &self,
        pool: &PoolInfo,
        block: BlockId,
    ) -> BoxFuture<'static, Result<PoolState>> {
        // Create pool contract directly from address
        let pool_contract = self.weighted_pool_at(pool.address);
        let fetch_dynamic_data = pool_contract
            .methods()
            .get_weighted_pool_dynamic_data()
            .block(block)
            .call();

        let pool = pool.clone();
        async move {
            let dynamic_data = fetch_dynamic_data.await?;
            let (balances_live_scaled18, _token_rates, static_swap_fee_percentage, _total_supply, _is_pool_initialized, is_pool_paused, _is_pool_in_recovery_mode) = dynamic_data;
            let paused = is_pool_paused;
            let swap_fee = Bfp::from_wei(static_swap_fee_percentage);
            let balances = balances_live_scaled18;

            // Ensure the number of balances matches the number of tokens
            ensure!(pool.tokens.len() == balances.len(), "pool token mismatch: expected {} tokens, got {} balances", pool.tokens.len(), balances.len());

            let tokens = itertools::izip!(&pool.tokens, balances, &pool.scaling_factors)
                .map(|(&address, balance, &scaling_factor)| {
                    // Create a temporary TokenState to use the downscale method
                    let temp_token_state = TokenState {
                    balance,
                    scaling_factor,
                    };
                    let native_balance = temp_token_state.downscale_down(Bfp::from_wei(balance)).unwrap_or(U256::zero());
                    (
                        address,
                        TokenState {
                            balance: native_balance, // Store native decimals!
                            scaling_factor,
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

            Ok(PoolStatus::Active(Pool {
                id: pool_id,
                kind: pool_state.into(),
            }))
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
                graph_api::{Token, GqlChain, DynamicData, PoolData},
                pools::{MockFactoryIndexing, PoolKind, weighted},
            },
            token_info::{MockTokenInfoFetching, TokenInfo},
        },
        ethcontract::U256,
        ethcontract_mock::Mock,
        futures::future,
        maplit::{btreemap, hashmap},
    };

    #[tokio::test]
    async fn fetch_common_pool_info() {
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        pool.expect_call(BalancerV3WeightedPool::signatures().get_tokens())
            .returns(tokens.to_vec());

        let vault = mock.deploy(BalancerV3Vault::raw_contract().interface.abi.clone());

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
            vault: BalancerV3Vault::at(&web3, vault.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = pool_info_fetcher
            .fetch_common_pool_info(pool.address(), 1337)
            .await
            .unwrap();

        assert_eq!(
            pool_info,
            PoolInfo {
                id: pool.address(),
                address: pool.address(),
                tokens: tokens.to_vec(),
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(12)],
                block_created: 1337,
            }
        );
    }

    #[tokio::test]
    async fn fetch_common_pool_state() {
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];
        let balances = [bfp_v3!("1000.0"), bfp_v3!("10.0"), bfp_v3!("15.0")];
        let scaling_factors = [Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(12)];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let mock_pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        mock_pool.expect_call(BalancerV3WeightedPool::signatures().get_weighted_pool_dynamic_data())
            .returns((
                balances.into_iter().map(Bfp::as_uint256).collect(),
                vec![U256::zero(), U256::zero(), U256::zero()],
                bfp_v3!("0.003").as_uint256(),
                U256::zero(),
                true,
                false,
                false,
            ));

        let vault = mock.deploy(BalancerV3Vault::raw_contract().interface.abi.clone());

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, vault.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = PoolInfo {
            id: mock_pool.address(),
            address: mock_pool.address(),
            tokens: tokens.to_vec(),
            scaling_factors: scaling_factors.to_vec(),
            block_created: 1337,
        };

        let pool_state = {
            let block = web3.eth().block_number().await.unwrap();

            let pool_state = pool_info_fetcher.fetch_common_pool_state(&pool_info, block.into());

            pool_state.await.unwrap()
        };

        assert_eq!(
            pool_state,
            PoolState {
                paused: false,
                swap_fee: bfp_v3!("0.003"),
                tokens: btreemap! {
                    tokens[0] => TokenState {
                        balance: balances[0].as_uint256(),
                        scaling_factor: scaling_factors[0],
                    },
                    tokens[1] => TokenState {
                        balance: balances[1].as_uint256(),
                        scaling_factor: scaling_factors[1],
                    },
                    tokens[2] => TokenState {
                        // Downscale 15e18 by 1e12 -> 15e6
                        balance: U256::from(15_000_000u64),
                        scaling_factor: scaling_factors[2],
                    },
                },
            }
        );
    }

    #[tokio::test]
    async fn fetch_state_errors_on_token_mismatch() {
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let mock_pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        mock_pool.expect_call(BalancerV3WeightedPool::signatures().get_weighted_pool_dynamic_data())
            .returns((
                vec![U256::zero(), U256::zero()], // Only 2 balances instead of 3
                vec![U256::zero(), U256::zero()],
                U256::zero(),
                U256::zero(),
                true,
                false,
                false,
            ));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, H160::zero()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = PoolInfo {
            id: mock_pool.address(),
            address: mock_pool.address(),
            tokens: tokens.to_vec(),
            scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(0)],
            block_created: 1337,
        };

        let block = web3.eth().block_number().await.unwrap();
        let result = pool_info_fetcher
            .fetch_common_pool_state(&pool_info, block.into())
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_specialized_pool_state() {
        let tokens = [H160([1; 20]), H160([2; 20])];
        let balances = [bfp_v3!("1000.0"), bfp_v3!("10.0")];
        let scaling_factors = [Bfp::exp10(0), Bfp::exp10(0)];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let mock_pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        mock_pool.expect_call(BalancerV3WeightedPool::signatures().get_weighted_pool_dynamic_data())
            .returns((
                balances.into_iter().map(Bfp::as_uint256).collect(),
                vec![U256::zero(), U256::zero()],
                bfp_v3!("0.003").as_uint256(),
                U256::zero(),
                true,
                false,
                false,
            ));

        let mut mock_factory = MockFactoryIndexing::new();
        let tokens_clone = tokens;
        let balances_clone = balances;
        mock_factory
            .expect_fetch_pool_state()
            .returning(move |_, _, _| {
                Box::pin(future::ok(Some(weighted::PoolState {
                    tokens: btreemap! {
                        tokens_clone[0] => weighted::TokenState {
                            common: TokenState {
                                balance: balances_clone[0].as_uint256(),
                                scaling_factor: Bfp::exp10(0),
                            },
                            weight: bfp_v3!("0.5"),
                        },
                        tokens_clone[1] => weighted::TokenState {
                            common: TokenState {
                                balance: balances_clone[1].as_uint256(),
                                scaling_factor: Bfp::exp10(0),
                            },
                            weight: bfp_v3!("0.5"),
                        },
                    },
                    swap_fee: bfp_v3!("0.003"),
                    version: weighted::Version::V1,
                })))
            });

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, H160::zero()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: mock_pool.address(),
                address: mock_pool.address(),
                tokens: tokens.to_vec(),
                scaling_factors: scaling_factors.to_vec(),
                block_created: 1337,
            },
            weights: vec![bfp_v3!("0.5"), bfp_v3!("0.5")],
        };

        let pool_status = {
            let block = web3.eth().block_number().await.unwrap();
            pool_info_fetcher.fetch_pool(&pool_info, block.into()).await.unwrap()
        };

        match pool_status {
            PoolStatus::Active(pool) => {
                assert_eq!(pool.id, pool_info.common.address);
                match pool.kind {
                    PoolKind::Weighted(state) => {
                        assert_eq!(state.tokens.len(), 2);
                        assert_eq!(state.swap_fee, bfp_v3!("0.003"));
                    }
                    PoolKind::Stable(_) => {
                        // Stable pools are not tested in this specific test
                        // This is just to handle the exhaustive pattern matching
                    }
                }
            }
            _ => panic!("expected active pool"),
        }
    }

    #[tokio::test]
    async fn fetch_specialized_pool_state_for_paused_pool() {
        let mock = Mock::new(42);
        let web3 = mock.web3();

        let mock_pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        mock_pool.expect_call(BalancerV3WeightedPool::signatures().get_weighted_pool_dynamic_data())
            .returns((
                vec![bfp_v3!("1000.0").as_uint256(), bfp_v3!("10.0").as_uint256()],
                vec![U256::zero(), U256::zero()],
                bfp_v3!("0.003").as_uint256(),
                U256::zero(),
                true,
                true, // Pool is paused
                false,
            ));

        let mut mock_factory = MockFactoryIndexing::new();
        mock_factory
            .expect_fetch_pool_state()
            .returning(|_, _, _| Box::pin(future::ok(Some(weighted::PoolState {
                tokens: btreemap! {},
                swap_fee: bfp_v3!("0.003"),
                version: weighted::Version::V1,
            }))));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, H160::zero()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: mock_pool.address(),
                address: mock_pool.address(),
                tokens: vec![H160([1; 20]), H160([2; 20])],
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                block_created: 1337,
            },
            weights: vec![bfp_v3!("0.5"), bfp_v3!("0.5")],
        };

        let pool_status = {
            let block = web3.eth().block_number().await.unwrap();
            pool_info_fetcher.fetch_pool(&pool_info, block.into()).await.unwrap()
        };

        match pool_status {
            PoolStatus::Paused => {}
            _ => panic!("expected paused pool"),
        }
    }

    #[tokio::test]
    async fn fetch_specialized_pool_state_for_disabled_pool() {
        let tokens = [H160([1; 20]), H160([2; 20])];
        let balances = [bfp_v3!("1000.0"), bfp_v3!("10.0")];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let mock_pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        mock_pool.expect_call(BalancerV3WeightedPool::signatures().get_weighted_pool_dynamic_data())
            .returns((
                balances.into_iter().map(|b| b.as_uint256()).collect(),
                vec![U256::zero(), U256::zero()],
                bfp_v3!("0.003").as_uint256(),
                U256::zero(),
                true,
                false,
                false,
            ));

        let mut mock_factory = MockFactoryIndexing::new();
        mock_factory
            .expect_fetch_pool_state()
            .returning(|_, _, _| Box::pin(future::ok(None)));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, H160::zero()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: mock_pool.address(),
                address: mock_pool.address(),
                tokens: tokens.to_vec(),
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                block_created: 1337,
            },
            weights: vec![bfp_v3!("0.5"), bfp_v3!("0.5")],
        };

        let pool_status = {
            let block = web3.eth().block_number().await.unwrap();
            pool_info_fetcher.fetch_pool(&pool_info, block.into()).await.unwrap()
        };

        match pool_status {
            PoolStatus::Disabled => {}
            _ => panic!("expected disabled pool"),
        }
    }

    #[tokio::test]
    async fn scaling_factor_error_on_missing_info() {
        let tokens = [H160([1; 20]), H160([2; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        pool.expect_call(BalancerV3WeightedPool::signatures().get_tokens())
            .returns(tokens.to_vec());

        let vault = mock.deploy(BalancerV3Vault::raw_contract().interface.abi.clone());

        let mut token_infos = MockTokenInfoFetching::new();
        token_infos
            .expect_get_token_infos()
            .returning(|_| hashmap! {});

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, vault.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };

        let result = pool_info_fetcher.fetch_common_pool_info(pool.address(), 1337).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn scaling_factor_error_on_missing_decimals() {
        let tokens = [H160([1; 20]), H160([2; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let pool = mock.deploy(BalancerV3WeightedPool::raw_contract().interface.abi.clone());
        pool.expect_call(BalancerV3WeightedPool::signatures().get_tokens())
            .returns(tokens.to_vec());

        let vault = mock.deploy(BalancerV3Vault::raw_contract().interface.abi.clone());

        let mut token_infos = MockTokenInfoFetching::new();
        token_infos.expect_get_token_infos().returning(|tokens| {
            tokens
                .iter()
                .map(|&token| {
                    (
                        token,
                        TokenInfo {
                            decimals: None, // Missing decimals
                            symbol: None,
                        },
                    )
                })
                .collect()
        });

        let pool_info_fetcher = PoolInfoFetcher {
            vault: BalancerV3Vault::at(&web3, vault.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };

        let result = pool_info_fetcher.fetch_common_pool_info(pool.address(), 1337).await;
        assert!(result.is_err());
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
                },
                Token {
                    address: H160([0x44; 20]),
                    decimals: 6,
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
                },
            ],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 1234567890,
        };

        let pool_info = PoolInfo::from_graph_data(&pool, 42).unwrap();

        assert_eq!(pool_info.id, H160([0x22; 20])); // For V3, pool ID is the pool address
        assert_eq!(pool_info.address, H160([0x22; 20]));
        assert_eq!(pool_info.tokens, vec![H160([0x33; 20]), H160([0x44; 20])]);
        assert_eq!(pool_info.scaling_factors, vec![Bfp::exp10(0), Bfp::exp10(12)]);
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
            pool_tokens: vec![
                Token {
                    address: H160([0x33; 20]),
                    decimals: 18,
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
                },
            ],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 1234567890,
        };

        let result = PoolInfo::from_graph_data(&pool, 42);
        assert!(result.is_err());
    }

    #[test]
    fn pool_conversion_invalid_decimals() {
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
                    decimals: 19, // Invalid: > 18
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
                },
                Token {
                    address: H160([0x44; 20]),
                    decimals: 6,
                    weight: Some(Bfp::from_wei(U256::from(500_000_000_000_000_000u128))),
                },
            ],
            dynamic_data: DynamicData { swap_enabled: true },
            create_time: 1234567890,
        };

        let result = PoolInfo::from_graph_data(&pool, 42);
        assert!(result.is_err());
    }

    #[test]
    fn scaling_factor_from_decimals_ok_and_err() {
        assert_eq!(
            scaling_factor_from_decimals(18).unwrap(),
            Bfp::exp10(0)
        );
        assert_eq!(
            scaling_factor_from_decimals(6).unwrap(),
            Bfp::exp10(12)
        );
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
        let (_shared_fut, shared_rx) = share_common_pool_state(future::pending::<Result<PoolState>>());

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
        let (shared_fut, shared_rx) = share_common_pool_state(future::err::<PoolState, _>(anyhow!("test error")));

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