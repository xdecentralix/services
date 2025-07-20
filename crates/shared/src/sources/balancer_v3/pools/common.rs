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
    contracts::{BalancerV3VaultExtension},
    ethcontract::{BlockId, H160, U256},
    futures::{FutureExt as _, TryFutureExt, future::BoxFuture},
    std::{collections::BTreeMap, future::Future, sync::Arc},
    tokio::sync::oneshot,
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
    vault_extension: BalancerV3VaultExtension,
    factory: Factory,
    token_infos: Arc<dyn TokenInfoFetching>,
}

impl<Factory> PoolInfoFetcher<Factory> {
    pub fn new(
        vault_extension: BalancerV3VaultExtension,
        factory: Factory,
        token_infos: Arc<dyn TokenInfoFetching>,
    ) -> Self {
        Self {
            vault_extension,
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
        // For V3, pool_id is the pool address itself (H160)
        let pool_id = pool_address;
        let (tokens, _, _, _) = self
            .vault_extension
            .methods()
            .get_pool_token_info(pool_address)
            .call()
            .await?;
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
        let fetch_paused = self
            .vault_extension
            .methods()
            .get_pool_paused_state(pool.address)
            .block(block)
            .call()
            .map_ok(|result| result.0);
        let fetch_swap_fee = self
            .vault_extension
            .methods()
            .get_static_swap_fee_percentage(pool.address)
            .block(block)
            .call();
        let fetch_balances = self
            .vault_extension
            .methods()
            .get_pool_token_info(pool.address)
            .block(block)
            .call();

        // Because of a `mockall` limitation, we **need** the future returned
        // here to be `'static`. This requires us to clone and move `pool` into
        // the async closure - otherwise it would only live for as long as
        // `pool`, i.e. `'_`.
        let pool = pool.clone();
        async move {
            let (paused, swap_fee, balances) =
                futures::try_join!(fetch_paused, fetch_swap_fee, fetch_balances)?;
            let swap_fee = Bfp::from_wei(swap_fee);

            let (token_addresses, _, balances_raw, _) = balances;
            ensure!(pool.tokens == token_addresses, "pool token mismatch");
            let tokens = itertools::izip!(&pool.tokens, balances_raw, &pool.scaling_factors)
                .map(|(&address, balance, &scaling_factor)| {
                    (
                        address,
                        TokenState {
                            balance,
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
        anyhow::bail,
        contracts::{BalancerV3WeightedPool, dummy_contract},
        ethcontract::U256,
        ethcontract_mock::Mock,
        futures::future,
        maplit::{btreemap, hashmap},
        mockall::predicate,
    };

    #[tokio::test]
    async fn fetch_common_pool_info() {
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((tokens.to_vec(), vec![], vec![], vec![]));

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
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = pool_info_fetcher
            .fetch_common_pool_info(pool_id, 1337)
            .await
            .unwrap();

        assert_eq!(
            pool_info,
            PoolInfo {
                id: pool_id,
                address: pool_id,
                tokens: tokens.to_vec(),
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(12)],
                block_created: 1337,
            }
        );
    }

    #[tokio::test]
    async fn fetch_common_pool_state() {
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];
        let balances = [bfp!("1000.0"), bfp!("10.0"), bfp!("15.0")];
        let scaling_factors = [Bfp::exp10(0), Bfp::exp10(0), Bfp::exp10(12)];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_paused_state())
            .predicate((predicate::eq(pool_id),))
            .returns((false, 0.into(), 0.into(), H160::zero()));
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_static_swap_fee_percentage())
            .predicate((predicate::eq(pool_id),))
            .returns(bfp!("0.003").as_uint256());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((
                tokens.to_vec(),
                vec![],
                balances.into_iter().map(Bfp::as_uint256).collect(),
                vec![],
            ));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = PoolInfo {
            id: pool_id,
            address: pool_id,
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
                swap_fee: bfp!("0.003"),
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
                        balance: balances[2].as_uint256(),
                        scaling_factor: scaling_factors[2],
                    },
                },
            }
        );
    }

    #[tokio::test]
    async fn fetch_state_errors_on_token_mismatch() {
        let tokens = [H160([1; 20]), H160([2; 20]), H160([3; 20])];
        let pool_id = H160::zero();

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_paused_state())
            .predicate((predicate::eq(pool_id),))
            .returns((false, 0.into(), 0.into(), H160::zero()));
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_static_swap_fee_percentage())
            .predicate((predicate::eq(pool_id),))
            .returns(0.into());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((
                vec![H160([1; 20]), H160([4; 20])],
                vec![],
                vec![0.into(), 0.into()],
                vec![],
            ));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };
        let pool_info = PoolInfo {
            id: pool_id,
            address: pool_id,
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
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20])];
        let balances = [bfp!("1000.0"), bfp!("10.0")];
        let scaling_factors = [Bfp::exp10(0), Bfp::exp10(0)];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_paused_state())
            .predicate((predicate::eq(pool_id),))
            .returns((false, 0.into(), 0.into(), H160::zero()));
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_static_swap_fee_percentage())
            .predicate((predicate::eq(pool_id),))
            .returns(bfp!("0.003").as_uint256());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((
                tokens.to_vec(),
                vec![],
                balances.into_iter().map(Bfp::as_uint256).collect(),
                vec![],
            ));

        let mut mock_factory = MockFactoryIndexing::new();
        mock_factory
            .expect_fetch_pool_state()
            .returning(|_, _, _| {
                Box::pin(future::ok(Some(weighted::PoolState {
                    tokens: btreemap! {
                        tokens[0] => weighted::TokenState {
                            balance: balances[0].as_uint256(),
                            weight: bfp!("0.5"),
                        },
                        tokens[1] => weighted::TokenState {
                            balance: balances[1].as_uint256(),
                            weight: bfp!("0.5"),
                        },
                    },
                    swap_fee: bfp!("0.003"),
                })))
            });

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: pool_id,
                address: pool_id,
                tokens: tokens.to_vec(),
                scaling_factors: scaling_factors.to_vec(),
                block_created: 1337,
            },
            weights: vec![bfp!("0.5"), bfp!("0.5")],
        };

        let pool_status = {
            let block = web3.eth().block_number().await.unwrap();
            pool_info_fetcher.fetch_pool(&pool_info, block.into()).await.unwrap()
        };

        match pool_status {
            PoolStatus::Active(pool) => {
                assert_eq!(pool.id, pool_id);
                match pool.kind {
                    PoolKind::Weighted(state) => {
                        assert_eq!(state.tokens.len(), 2);
                        assert_eq!(state.swap_fee, bfp!("0.003"));
                    }
                }
            }
            _ => panic!("expected active pool"),
        }
    }

    #[tokio::test]
    async fn fetch_specialized_pool_state_for_paused_pool() {
        let pool_id = H160([0x90; 20]);

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_paused_state())
            .predicate((predicate::eq(pool_id),))
            .returns((true, 0.into(), 0.into(), H160::zero()));

        let mut mock_factory = MockFactoryIndexing::new();
        mock_factory
            .expect_fetch_pool_state()
            .returning(|_, _, _| Box::pin(future::ok(Some(weighted::PoolState {
                tokens: btreemap! {},
                swap_fee: bfp!("0.003"),
            }))));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: pool_id,
                address: pool_id,
                tokens: vec![H160([1; 20]), H160([2; 20])],
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                block_created: 1337,
            },
            weights: vec![bfp!("0.5"), bfp!("0.5")],
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
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20])];
        let balances = [bfp!("1000.0"), bfp!("10.0")];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_paused_state())
            .predicate((predicate::eq(pool_id),))
            .returns((false, 0.into(), 0.into(), H160::zero()));
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_static_swap_fee_percentage())
            .predicate((predicate::eq(pool_id),))
            .returns(bfp!("0.003").as_uint256());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((
                tokens.to_vec(),
                vec![],
                balances.into_iter().map(Bfp::as_uint256).collect(),
                vec![],
            ));

        let mut mock_factory = MockFactoryIndexing::new();
        mock_factory
            .expect_fetch_pool_state()
            .returning(|_, _, _| Box::pin(future::ok(None)));

        let token_infos = MockTokenInfoFetching::new();

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: mock_factory,
            token_infos: Arc::new(token_infos),
        };
        let pool_info = weighted::PoolInfo {
            common: PoolInfo {
                id: pool_id,
                address: pool_id,
                tokens: tokens.to_vec(),
                scaling_factors: vec![Bfp::exp10(0), Bfp::exp10(0)],
                block_created: 1337,
            },
            weights: vec![bfp!("0.5"), bfp!("0.5")],
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
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((tokens.to_vec(), vec![], vec![], vec![]));

        let mut token_infos = MockTokenInfoFetching::new();
        token_infos
            .expect_get_token_infos()
            .returning(|_| btreemap! {});

        let pool_info_fetcher = PoolInfoFetcher {
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };

        let result = pool_info_fetcher.fetch_common_pool_info(pool_id, 1337).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn scaling_factor_error_on_missing_decimals() {
        let pool_id = H160([0x90; 20]);
        let tokens = [H160([1; 20]), H160([2; 20])];

        let mock = Mock::new(42);
        let web3 = mock.web3();

        let vault_extension = mock.deploy(BalancerV3VaultExtension::raw_contract().interface.abi.clone());
        vault_extension
            .expect_call(BalancerV3VaultExtension::signatures().get_pool_token_info())
            .predicate((predicate::eq(pool_id),))
            .returns((tokens.to_vec(), vec![], vec![], vec![]));

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
            vault_extension: BalancerV3VaultExtension::at(&web3, vault_extension.address()),
            factory: MockFactoryIndexing::new(),
            token_infos: Arc::new(token_infos),
        };

        let result = pool_info_fetcher.fetch_common_pool_info(pool_id, 1337).await;
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

        let (result1, result2) = futures::try_join!(
            shared_rx,
            shared_fut.map(|_| ())
        )
        .unwrap();

        assert_eq!(result1, result2.unwrap());
    }

    #[tokio::test]
    #[should_panic(expected = "sender dropped")]
    async fn shared_pool_state_future_panics_if_pending() {
        let (shared_fut, shared_rx) = share_common_pool_state(future::pending::<Result<PoolState>>());

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
        shared_fut.await;
    }

    #[tokio::test]
    async fn share_pool_state_future_if_errored() {
        let (shared_fut, shared_rx) = share_common_pool_state(future::err::<PoolState, _>(anyhow!("test error")));

        let (result1, result2) = futures::try_join!(
            shared_rx,
            shared_fut.map(|_| ())
        )
        .unwrap();

        assert!(result1.is_err());
        assert!(result2.is_ok());
    }

    #[test]
    fn compute_scaling_rates() {
        let scaling_factor = Bfp::from_wei(U256::exp10(12)); // 12 decimals
        let scaling_rate = compute_scaling_rate(scaling_factor).unwrap();
        assert_eq!(scaling_rate, U256::exp10(6)); // 18 - 12 = 6

        let scaling_factor = Bfp::from_wei(U256::exp10(18)); // 18 decimals
        let scaling_rate = compute_scaling_rate(scaling_factor).unwrap();
        assert_eq!(scaling_rate, U256::exp10(0)); // 18 - 18 = 0
    }
} 