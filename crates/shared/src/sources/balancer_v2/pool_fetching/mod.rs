//! Pool Fetching is primarily concerned with retrieving relevant pools from the
//! `BalancerPoolRegistry` when given a collection of `TokenPair`. Each of these
//! pools are then queried for their `token_balances` and the `PoolFetcher`
//! returns all up-to-date `Weighted` and `Stable` pools to be consumed by
//! external users (e.g. Price Estimators and Solvers).

use {
    self::{
        aggregate::Aggregate,
        cache::Cache,
        internal::InternalPoolFetching,
        registry::Registry,
    },
    super::{
        graph_api::{BalancerApiClient, GqlChain, RegisteredPools},
        pool_init::PoolInitializing,
        pools::{
            FactoryIndexing,
            Pool,
            PoolIndexing,
            PoolKind,
            common::{self, PoolInfoFetcher},
            gyro_2clp,
            gyro_3clp,
            gyro_e,
            stable,
            weighted,
        },
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
    crate::{
        ethrpc::Web3,
        recent_block_cache::{Block, CacheConfig},
        token_info::TokenInfoFetching,
    },
    anyhow::{Context, Result},
    clap::ValueEnum,
    contracts::alloy::{
        BalancerV2ComposableStablePoolFactory,
        BalancerV2ComposableStablePoolFactoryV3,
        BalancerV2ComposableStablePoolFactoryV4,
        BalancerV2ComposableStablePoolFactoryV5,
        BalancerV2ComposableStablePoolFactoryV6,
        BalancerV2Gyro2CLPPoolFactory,
        BalancerV2Gyro3CLPPoolFactory,
        BalancerV2GyroECLPPoolFactory,
        BalancerV2LiquidityBootstrappingPoolFactory,
        BalancerV2NoProtocolFeeLiquidityBootstrappingPoolFactory,
        BalancerV2StablePoolFactoryV2,
        BalancerV2Vault,
        BalancerV2WeightedPool2TokensFactory,
        BalancerV2WeightedPoolFactory,
        BalancerV2WeightedPoolFactoryV3,
        BalancerV2WeightedPoolFactoryV4,
        InstanceExt,
        Provider as DynProvider,
    },
    ethcontract::{BlockId, H160, H256},
    ethrpc::block_stream::{BlockRetrieving, CurrentBlockWatcher},
    model::TokenPair,
    reqwest::{Client, Url},
    std::{
        collections::{BTreeMap, HashSet},
        sync::Arc,
    },
    tracing::instrument,
};
pub use {
    common::TokenState,
    gyro_2clp::Version as Gyro2CLPPoolVersion,
    gyro_3clp::Version as Gyro3CLPPoolVersion,
    gyro_e::Version as GyroEPoolVersion,
    stable::AmplificationParameter,
    weighted::{TokenState as WeightedTokenState, Version as WeightedPoolVersion},
};

mod aggregate;
mod cache;
mod internal;
mod pool_storage;
mod registry;

pub trait BalancerPoolEvaluating {
    fn properties(&self) -> CommonPoolState;
}

#[derive(Clone, Debug)]
pub struct CommonPoolState {
    pub id: H256,
    pub address: H160,
    pub swap_fee: Bfp,
    pub paused: bool,
}

#[derive(Clone, Debug)]
pub struct WeightedPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, WeightedTokenState>,
    pub version: WeightedPoolVersion,
}

impl WeightedPool {
    pub fn new_unpaused(pool_id: H256, weighted_state: weighted::PoolState) -> Self {
        WeightedPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_address_from_id(pool_id),
                swap_fee: weighted_state.swap_fee,
                paused: false,
            },
            reserves: weighted_state.tokens.into_iter().collect(),
            version: weighted_state.version,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StablePool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, TokenState>,
    pub amplification_parameter: AmplificationParameter,
}

impl StablePool {
    pub fn new_unpaused(pool_id: H256, stable_state: stable::PoolState) -> Self {
        StablePool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_address_from_id(pool_id),
                swap_fee: stable_state.swap_fee,
                paused: false,
            },
            reserves: stable_state.tokens.into_iter().collect(),
            amplification_parameter: stable_state.amplification_parameter,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GyroEPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, TokenState>,
    pub version: GyroEPoolVersion,
    // Gyro E-CLP static parameters (immutable after pool creation)
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

impl GyroEPool {
    pub fn new_unpaused(pool_id: H256, gyro_e_state: gyro_e::PoolState) -> Self {
        GyroEPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_address_from_id(pool_id),
                swap_fee: gyro_e_state.swap_fee,
                paused: false,
            },
            reserves: gyro_e_state.tokens.into_iter().collect(),
            version: gyro_e_state.version,
            // Static parameters from PoolState
            params_alpha: gyro_e_state.params_alpha,
            params_beta: gyro_e_state.params_beta,
            params_c: gyro_e_state.params_c,
            params_s: gyro_e_state.params_s,
            params_lambda: gyro_e_state.params_lambda,
            tau_alpha_x: gyro_e_state.tau_alpha_x,
            tau_alpha_y: gyro_e_state.tau_alpha_y,
            tau_beta_x: gyro_e_state.tau_beta_x,
            tau_beta_y: gyro_e_state.tau_beta_y,
            u: gyro_e_state.u,
            v: gyro_e_state.v,
            w: gyro_e_state.w,
            z: gyro_e_state.z,
            d_sq: gyro_e_state.d_sq,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Gyro2CLPPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, TokenState>,
    pub version: Gyro2CLPPoolVersion,
    // Gyro 2-CLP static parameters (immutable after pool creation)
    pub sqrt_alpha: SBfp,
    pub sqrt_beta: SBfp,
}

impl Gyro2CLPPool {
    pub fn new_unpaused(pool_id: H256, gyro_2clp_state: gyro_2clp::PoolState) -> Self {
        Gyro2CLPPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_address_from_id(pool_id),
                swap_fee: gyro_2clp_state.swap_fee,
                paused: false,
            },
            reserves: gyro_2clp_state.tokens.into_iter().collect(),
            version: gyro_2clp_state.version,
            // Static parameters from PoolState
            sqrt_alpha: gyro_2clp_state.sqrt_alpha,
            sqrt_beta: gyro_2clp_state.sqrt_beta,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Gyro3CLPPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, TokenState>,
    pub version: Gyro3CLPPoolVersion,
    // Gyro 3-CLP static parameter (immutable after pool creation)
    pub root3_alpha: Bfp,
}

impl Gyro3CLPPool {
    pub fn new_unpaused(pool_id: H256, gyro_3clp_state: gyro_3clp::PoolState) -> Self {
        Gyro3CLPPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_address_from_id(pool_id),
                swap_fee: gyro_3clp_state.swap_fee,
                paused: false,
            },
            reserves: gyro_3clp_state.tokens.into_iter().collect(),
            version: gyro_3clp_state.version,
            // Static parameter from PoolState
            root3_alpha: gyro_3clp_state.root3_alpha,
        }
    }
}

#[derive(Default)]
pub struct FetchedBalancerPools {
    pub stable_pools: Vec<StablePool>,
    pub weighted_pools: Vec<WeightedPool>,
    pub gyro_2clp_pools: Vec<Gyro2CLPPool>,
    pub gyro_3clp_pools: Vec<Gyro3CLPPool>,
    pub gyro_e_pools: Vec<GyroEPool>,
}

impl FetchedBalancerPools {
    pub fn relevant_tokens(&self) -> HashSet<H160> {
        let mut tokens = HashSet::new();
        tokens.extend(
            self.stable_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.weighted_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.gyro_2clp_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.gyro_3clp_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.gyro_e_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens
    }
}

#[mockall::automock]
#[async_trait::async_trait]
pub trait BalancerPoolFetching: Send + Sync {
    async fn fetch(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<FetchedBalancerPools>;
}

pub struct BalancerPoolFetcher {
    fetcher: Arc<dyn InternalPoolFetching>,
    // We observed some balancer pools like https://app.balancer.fi/#/pool/0x072f14b85add63488ddad88f855fda4a99d6ac9b000200000000000000000027
    // being problematic because their token balance becomes out of sync leading to simulation
    // failures.
    // https://forum.balancer.fi/t/medium-severity-bug-found/3161
    pool_id_deny_list: Vec<H256>,
}

/// An enum containing all supported Balancer factory types.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, ValueEnum)]
#[clap(rename_all = "verbatim")]
pub enum BalancerFactoryKind {
    Weighted,
    WeightedV3,
    WeightedV4,
    Weighted2Token,
    StableV2,
    LiquidityBootstrapping,
    NoProtocolFeeLiquidityBootstrapping,
    ComposableStable,
    ComposableStableV3,
    ComposableStableV4,
    ComposableStableV5,
    ComposableStableV6,
    Gyro2CLP,
    Gyro3CLP,
    GyroE,
}

pub enum BalancerFactoryInstance {
    Weighted(BalancerV2WeightedPoolFactory::Instance),
    WeightedV3(BalancerV2WeightedPoolFactoryV3::Instance),
    WeightedV4(BalancerV2WeightedPoolFactoryV4::Instance),
    Weighted2Token(BalancerV2WeightedPool2TokensFactory::Instance),
    StableV2(BalancerV2StablePoolFactoryV2::Instance),
    LiquidityBootstrapping(BalancerV2LiquidityBootstrappingPoolFactory::Instance),
    NoProtocolFeeLiquidityBootstrapping(
        BalancerV2NoProtocolFeeLiquidityBootstrappingPoolFactory::Instance,
    ),
    ComposableStable(BalancerV2ComposableStablePoolFactory::Instance),
    ComposableStableV3(BalancerV2ComposableStablePoolFactoryV3::Instance),
    ComposableStableV4(BalancerV2ComposableStablePoolFactoryV4::Instance),
    ComposableStableV5(BalancerV2ComposableStablePoolFactoryV5::Instance),
    ComposableStableV6(BalancerV2ComposableStablePoolFactoryV6::Instance),
    Gyro2CLP(BalancerV2Gyro2CLPPoolFactory::Instance),
    Gyro3CLP(BalancerV2Gyro3CLPPoolFactory::Instance),
    GyroE(BalancerV2GyroECLPPoolFactory::Instance),
}

impl BalancerFactoryInstance {
    pub fn address(&self) -> &alloy::primitives::Address {
        match self {
            BalancerFactoryInstance::Weighted(instance) => instance.address(),
            BalancerFactoryInstance::WeightedV3(instance) => instance.address(),
            BalancerFactoryInstance::WeightedV4(instance) => instance.address(),
            BalancerFactoryInstance::Weighted2Token(instance) => instance.address(),
            BalancerFactoryInstance::StableV2(instance) => instance.address(),
            BalancerFactoryInstance::LiquidityBootstrapping(instance) => instance.address(),
            BalancerFactoryInstance::NoProtocolFeeLiquidityBootstrapping(instance) => {
                instance.address()
            }
            BalancerFactoryInstance::ComposableStable(instance) => instance.address(),
            BalancerFactoryInstance::ComposableStableV3(instance) => instance.address(),
            BalancerFactoryInstance::ComposableStableV4(instance) => instance.address(),
            BalancerFactoryInstance::ComposableStableV5(instance) => instance.address(),
            BalancerFactoryInstance::ComposableStableV6(instance) => instance.address(),
            BalancerFactoryInstance::Gyro2CLP(instance) => instance.address(),
            BalancerFactoryInstance::Gyro3CLP(instance) => instance.address(),
            BalancerFactoryInstance::GyroE(instance) => instance.address(),
        }
    }

    pub fn provider(&self) -> &DynProvider {
        match self {
            BalancerFactoryInstance::Weighted(instance) => instance.provider(),
            BalancerFactoryInstance::WeightedV3(instance) => instance.provider(),
            BalancerFactoryInstance::WeightedV4(instance) => instance.provider(),
            BalancerFactoryInstance::Weighted2Token(instance) => instance.provider(),
            BalancerFactoryInstance::StableV2(instance) => instance.provider(),
            BalancerFactoryInstance::LiquidityBootstrapping(instance) => instance.provider(),
            BalancerFactoryInstance::NoProtocolFeeLiquidityBootstrapping(instance) => {
                instance.provider()
            }
            BalancerFactoryInstance::ComposableStable(instance) => instance.provider(),
            BalancerFactoryInstance::ComposableStableV3(instance) => instance.provider(),
            BalancerFactoryInstance::ComposableStableV4(instance) => instance.provider(),
            BalancerFactoryInstance::ComposableStableV5(instance) => instance.provider(),
            BalancerFactoryInstance::ComposableStableV6(instance) => instance.provider(),
            BalancerFactoryInstance::Gyro2CLP(instance) => instance.provider(),
            BalancerFactoryInstance::Gyro3CLP(instance) => instance.provider(),
            BalancerFactoryInstance::GyroE(instance) => instance.provider(),
        }
    }
}

impl BalancerFactoryKind {
    /// Returns a vector with supported factories for the specified chain ID.
    pub fn for_chain(chain_id: u64) -> Vec<Self> {
        match chain_id {
            1 => vec![
                // Mainnet
                Self::Weighted,
                Self::WeightedV3,
                Self::WeightedV4,
                Self::Weighted2Token,
                Self::StableV2,
                Self::LiquidityBootstrapping,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStable,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::GyroE,
            ],
            10 => vec![
                // Optimism
                Self::Weighted,
                Self::WeightedV3,
                Self::WeightedV4,
                Self::Weighted2Token,
                Self::StableV2,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStable,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::GyroE,
            ],
            56 => vec![
                // BNB
                Self::WeightedV3,
                Self::WeightedV4,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStable,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
            ],
            100 => vec![
                // Gnosis
                Self::WeightedV3,
                Self::WeightedV4,
                Self::StableV2,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::GyroE,
            ],
            137 => vec![
                // Polygon
                Self::Weighted,
                Self::WeightedV3,
                Self::WeightedV4,
                Self::Weighted2Token,
                Self::StableV2,
                Self::LiquidityBootstrapping,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStable,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::Gyro2CLP,
                Self::Gyro3CLP,
                Self::GyroE,
            ],
            8453 => vec![
                // Base
                Self::WeightedV4,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::GyroE,
            ],
            42161 => vec![
                // Arbitrum One
                Self::Weighted,
                Self::WeightedV3,
                Self::WeightedV4,
                Self::Weighted2Token,
                Self::StableV2,
                Self::LiquidityBootstrapping,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStable,
                Self::ComposableStableV3,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::Gyro2CLP,
                Self::GyroE,
            ],
            43114 => vec![
                // Avalanche
                Self::WeightedV3,
                Self::WeightedV4,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
                Self::GyroE,
            ],
            11155111 => vec![
                // Sepolia
                Self::WeightedV4,
                Self::NoProtocolFeeLiquidityBootstrapping,
                Self::ComposableStableV4,
                Self::ComposableStableV5,
                Self::ComposableStableV6,
            ],
            _ => Default::default(),
        }
    }
}

/// All balancer related contracts that we expect to exist.
pub struct BalancerContracts {
    pub vault: BalancerV2Vault::Instance,
    pub factories: Vec<BalancerFactoryInstance>,
}

impl BalancerContracts {
    pub async fn try_new(
        web3_provider: &Web3,
        factory_kinds: Vec<BalancerFactoryKind>,
    ) -> Result<Self> {
        let web3_client =
            ethrpc::instrumented::instrument_with_label(web3_provider, "balancerV2".into());
        let vault = BalancerV2Vault::Instance::deployed(&web3_client.alloy)
            .await
            .context("Cannot retrieve balancer vault")?;

        macro_rules! instance {
            ($factory:ident) => {{
                $factory::Instance::deployed(&web3_client.alloy)
                    .await
                    .context(format!(
                        "Cannot retrieve Balancer factory {}",
                        stringify!($factory)
                    ))?
            }};
        }

        let mut factories = Vec::new();
        for kind in factory_kinds {
            let instance = match &kind {
                BalancerFactoryKind::Weighted => {
                    BalancerFactoryInstance::Weighted(instance!(BalancerV2WeightedPoolFactory))
                }
                BalancerFactoryKind::WeightedV3 => {
                    BalancerFactoryInstance::WeightedV3(instance!(BalancerV2WeightedPoolFactoryV3))
                }
                BalancerFactoryKind::WeightedV4 => {
                    BalancerFactoryInstance::WeightedV4(instance!(BalancerV2WeightedPoolFactoryV4))
                }
                BalancerFactoryKind::Weighted2Token => BalancerFactoryInstance::Weighted2Token(
                    instance!(BalancerV2WeightedPool2TokensFactory),
                ),
                BalancerFactoryKind::StableV2 => {
                    BalancerFactoryInstance::StableV2(instance!(BalancerV2StablePoolFactoryV2))
                }
                BalancerFactoryKind::LiquidityBootstrapping => {
                    BalancerFactoryInstance::LiquidityBootstrapping(instance!(
                        BalancerV2LiquidityBootstrappingPoolFactory
                    ))
                }
                BalancerFactoryKind::NoProtocolFeeLiquidityBootstrapping => {
                    BalancerFactoryInstance::NoProtocolFeeLiquidityBootstrapping(instance!(
                        BalancerV2NoProtocolFeeLiquidityBootstrappingPoolFactory
                    ))
                }
                BalancerFactoryKind::ComposableStable => BalancerFactoryInstance::ComposableStable(
                    instance!(BalancerV2ComposableStablePoolFactory),
                ),
                BalancerFactoryKind::ComposableStableV3 => {
                    BalancerFactoryInstance::ComposableStableV3(instance!(
                        BalancerV2ComposableStablePoolFactoryV3
                    ))
                }
                BalancerFactoryKind::ComposableStableV4 => {
                    BalancerFactoryInstance::ComposableStableV4(instance!(
                        BalancerV2ComposableStablePoolFactoryV4
                    ))
                }
                BalancerFactoryKind::ComposableStableV5 => {
                    BalancerFactoryInstance::ComposableStableV5(instance!(
                        BalancerV2ComposableStablePoolFactoryV5
                    ))
                }
                BalancerFactoryKind::ComposableStableV6 => {
                    BalancerFactoryInstance::ComposableStableV6(instance!(
                        BalancerV2ComposableStablePoolFactoryV6
                    ))
                }
                BalancerFactoryKind::Gyro2CLP => {
                    BalancerFactoryInstance::Gyro2CLP(instance!(BalancerV2Gyro2CLPPoolFactory))
                }
                BalancerFactoryKind::Gyro3CLP => {
                    BalancerFactoryInstance::Gyro3CLP(instance!(BalancerV2Gyro3CLPPoolFactory))
                }
                BalancerFactoryKind::GyroE => {
                    BalancerFactoryInstance::GyroE(instance!(BalancerV2GyroECLPPoolFactory))
                }
            };

            factories.push(instance);
        }

        Ok(Self { vault, factories })
    }
}

impl BalancerPoolFetcher {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        subgraph_url: &Url,
        block_retriever: Arc<dyn BlockRetrieving>,
        token_infos: Arc<dyn TokenInfoFetching>,
        config: CacheConfig,
        block_stream: CurrentBlockWatcher,
        client: Client,
        web3: Web3,
        contracts: &BalancerContracts,
        deny_listed_pool_ids: Vec<H256>,
        chain: GqlChain,
    ) -> Result<Self> {
        let pool_initializer = BalancerApiClient::from_subgraph_url(subgraph_url, client, chain)?;
        let web3 = ethrpc::instrumented::instrument_with_label(&web3, "balancerV2".into());
        let fetcher = Arc::new(Cache::new(
            create_aggregate_pool_fetcher(
                web3,
                pool_initializer,
                block_retriever,
                token_infos,
                contracts,
            )
            .await?,
            config,
            block_stream,
        )?);

        Ok(Self {
            fetcher,
            pool_id_deny_list: deny_listed_pool_ids,
        })
    }

    async fn fetch_pools(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<Vec<Pool>> {
        let mut pool_ids = self.fetcher.pool_ids_for_token_pairs(token_pairs).await;
        for id in &self.pool_id_deny_list {
            pool_ids.remove(id);
        }
        let pools = self.fetcher.pools_by_id(pool_ids, at_block).await?;

        Ok(pools)
    }
}

#[async_trait::async_trait]
impl BalancerPoolFetching for BalancerPoolFetcher {
    #[instrument(skip_all)]
    async fn fetch(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<FetchedBalancerPools> {
        let pools = self.fetch_pools(token_pairs, at_block).await?;

        // For now, split the `Vec<Pool>` into a `FetchedBalancerPools` to keep
        // compatibility with the rest of the project. This should eventually
        // be removed and we should use `balancer_v2::pools::Pool` everywhere
        // instead.
        let fetched_pools = pools.into_iter().fold(
            FetchedBalancerPools::default(),
            |mut fetched_pools, pool| {
                match pool.kind {
                    PoolKind::Weighted(state) => fetched_pools
                        .weighted_pools
                        .push(WeightedPool::new_unpaused(pool.id, state)),
                    PoolKind::Stable(state) => fetched_pools
                        .stable_pools
                        .push(StablePool::new_unpaused(pool.id, state)),
                    PoolKind::Gyro2CLP(state) => fetched_pools
                        .gyro_2clp_pools
                        .push(Gyro2CLPPool::new_unpaused(pool.id, state)),
                    PoolKind::Gyro3CLP(state) => fetched_pools
                        .gyro_3clp_pools
                        .push(Gyro3CLPPool::new_unpaused(pool.id, state)),
                    PoolKind::GyroE(state) => fetched_pools
                        .gyro_e_pools
                        .push(GyroEPool::new_unpaused(pool.id, *state)),
                }
                fetched_pools
            },
        );

        Ok(fetched_pools)
    }
}

/// Creates an aggregate fetcher for all supported pool factories.
async fn create_aggregate_pool_fetcher(
    web3: Web3,
    pool_initializer: impl PoolInitializing,
    block_retriever: Arc<dyn BlockRetrieving>,
    token_infos: Arc<dyn TokenInfoFetching>,
    contracts: &BalancerContracts,
) -> Result<Aggregate> {
    let registered_pools = pool_initializer.initialize_pools().await?;
    let fetched_block_number = registered_pools.fetched_block_number;
    let fetched_block_hash = web3
        .eth()
        .block(BlockId::Number(fetched_block_number.into()))
        .await?
        .context("failed to get block by block number")?
        .hash
        .context("missing hash from block")?;
    let mut registered_pools_by_factory = registered_pools.group_by_factory();

    macro_rules! registry {
        ($factory:ident, $instance:expr_2021) => {{
            use ethrpc::alloy::conversions::IntoLegacy;
            create_internal_pool_fetcher(
                contracts.vault.clone(),
                web3.clone(),
                $factory::Instance::new(*$instance.address(), $instance.provider().clone()),
                block_retriever.clone(),
                token_infos.clone(),
                $instance,
                registered_pools_by_factory
                    .remove(&(*$instance.address()).into_legacy())
                    .unwrap_or_else(|| RegisteredPools::empty(fetched_block_number)),
                fetched_block_hash,
            )?
        }};
    }

    let mut fetchers = Vec::new();
    for instance in &contracts.factories {
        let registry = match &instance {
            BalancerFactoryInstance::Weighted(_) => {
                registry!(BalancerV2WeightedPoolFactory, instance)
            }
            BalancerFactoryInstance::Weighted2Token(_) => {
                registry!(BalancerV2WeightedPoolFactory, instance)
            }
            BalancerFactoryInstance::WeightedV3(_) => {
                registry!(BalancerV2WeightedPoolFactoryV3, instance)
            }
            BalancerFactoryInstance::WeightedV4(_) => {
                registry!(BalancerV2WeightedPoolFactoryV3, instance)
            }
            BalancerFactoryInstance::StableV2(_) => {
                registry!(BalancerV2StablePoolFactoryV2, instance)
            }
            BalancerFactoryInstance::LiquidityBootstrapping(_) => {
                registry!(BalancerV2LiquidityBootstrappingPoolFactory, instance)
            }
            BalancerFactoryInstance::NoProtocolFeeLiquidityBootstrapping(_) => {
                registry!(BalancerV2LiquidityBootstrappingPoolFactory, instance)
            }
            BalancerFactoryInstance::ComposableStable(_) => {
                registry!(BalancerV2ComposableStablePoolFactory, instance)
            }
            BalancerFactoryInstance::ComposableStableV3(_) => {
                registry!(BalancerV2ComposableStablePoolFactory, instance)
            }
            BalancerFactoryInstance::ComposableStableV4(_) => {
                registry!(BalancerV2ComposableStablePoolFactory, instance)
            }
            BalancerFactoryInstance::ComposableStableV5(_) => {
                registry!(BalancerV2ComposableStablePoolFactory, instance)
            }
            BalancerFactoryInstance::ComposableStableV6(_) => {
                registry!(BalancerV2ComposableStablePoolFactory, instance)
            }
            BalancerFactoryInstance::Gyro2CLP(_) => {
                registry!(BalancerV2Gyro2CLPPoolFactory, instance)
            }
            BalancerFactoryInstance::Gyro3CLP(_) => {
                registry!(BalancerV2Gyro3CLPPoolFactory, instance)
            }
            BalancerFactoryInstance::GyroE(_) => {
                registry!(BalancerV2GyroECLPPoolFactory, instance)
            }
        };
        fetchers.push(registry);
    }

    // Just to catch cases where new Balancer factories get added for a pool
    // kind, but we don't index it, log a warning for unused pools.
    if !registered_pools_by_factory.is_empty() {
        let total_count = registered_pools_by_factory
            .values()
            .map(|registered| registered.pools.len())
            .sum::<usize>();
        let factories = registered_pools_by_factory
            .keys()
            .copied()
            .collect::<Vec<_>>();
        tracing::warn!(
            %total_count, ?factories,
            "found pools that don't correspond to any known Balancer pool factory",
        );
    }

    Ok(Aggregate::new(fetchers))
}

/// Helper method for creating a boxed `InternalPoolFetching` instance for the
/// specified factory and parameters.
fn create_internal_pool_fetcher<Factory>(
    vault: BalancerV2Vault::Instance,
    web3: Web3,
    factory: Factory,
    block_retriever: Arc<dyn BlockRetrieving>,
    token_infos: Arc<dyn TokenInfoFetching>,
    factory_instance: &BalancerFactoryInstance,
    registered_pools: RegisteredPools,
    fetched_block_hash: H256,
) -> Result<Box<dyn InternalPoolFetching>>
where
    Factory: FactoryIndexing,
{
    let initial_pools = registered_pools
        .pools
        .iter()
        .map(|pool| Factory::PoolInfo::from_graph_data(pool, registered_pools.fetched_block_number))
        .collect::<Result<_>>()?;
    let start_sync_at_block = Some((registered_pools.fetched_block_number, fetched_block_hash));

    Ok(Box::new(Registry::new(
        block_retriever,
        Arc::new(PoolInfoFetcher::new(vault, web3, factory, token_infos)),
        factory_instance,
        initial_pools,
        start_sync_at_block,
    )))
}

/// Extract the pool address from an ID.
///
/// This takes advantage that the first 20 bytes of the ID is the address of
/// the pool. For example the GNO-BAL pool with ID
/// `0x36128d5436d2d70cab39c9af9cce146c38554ff0000200000000000000000009`:
/// <https://etherscan.io/address/0x36128D5436d2d70cab39C9AF9CcE146C38554ff0>
fn pool_address_from_id(pool_id: H256) -> H160 {
    let mut address = H160::default();
    address.0.copy_from_slice(&pool_id.0[..20]);
    address
}

#[cfg(test)]
mod tests {
    use {super::*, hex_literal::hex};

    #[test]
    fn can_extract_address_from_pool_id() {
        assert_eq!(
            pool_address_from_id(H256(hex!(
                "36128d5436d2d70cab39c9af9cce146c38554ff0000200000000000000000009"
            ))),
            addr!("36128d5436d2d70cab39c9af9cce146c38554ff0"),
        );
    }
}
