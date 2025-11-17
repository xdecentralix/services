//! Pool Fetching is primarily concerned with retrieving relevant pools from the
//! `BalancerPoolRegistry` when given a collection of `TokenPair`. Each of these
//! pools are then queried for their `token_balances` and the `PoolFetcher`
//! returns all up-to-date `Weighted` pools to be consumed by external users
//! (e.g. Price Estimators and Solvers).

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
            gyro_e,
            quantamm,
            reclamm,
            stable,
            stable_surge,
            weighted,
        },
        swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    },
    crate::{
        ethrpc::{Web3, Web3Transport},
        recent_block_cache::{Block, CacheConfig},
        token_info::TokenInfoFetching,
    },
    anyhow::{Context, Result},
    clap::ValueEnum,
    contracts::{
        BalancerV3Gyro2CLPPoolFactory,
        BalancerV3GyroECLPPoolFactory,
        BalancerV3QuantAMMWeightedPoolFactory,
        BalancerV3ReClammPoolFactoryV2,
        BalancerV3StablePoolFactory,
        BalancerV3StablePoolFactoryV2,
        BalancerV3StableSurgePoolFactory,
        BalancerV3StableSurgePoolFactoryV2,
        BalancerV3Vault,
        BalancerV3WeightedPoolFactory,
        alloy::{BalancerV3BatchRouter, InstanceExt},
    },
    ethcontract::{BlockId, H160, H256, I256, Instance, U256, dyns::DynInstance},
    ethrpc::block_stream::{BlockRetrieving, CurrentBlockWatcher},
    model::TokenPair,
    reqwest::{Client, Url},
    std::{
        collections::{BTreeMap, HashSet},
        sync::Arc,
    },
};
pub use {
    common::TokenState,
    gyro_2clp::Version as Gyro2CLPPoolVersion,
    gyro_e::Version as GyroEPoolVersion,
    quantamm::{TokenState as QuantAmmTokenState, Version as QuantAmmPoolVersion},
    reclamm::Version as ReClammPoolVersion,
    stable::{
        AmplificationParameter,
        TokenState as StableTokenState,
        Version as StablePoolVersion,
    },
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
    pub id: H160,
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
    pub fn new_unpaused(pool_id: H160, weighted_state: weighted::PoolState) -> Self {
        WeightedPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id, // V3 pools are contract addresses
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
    pub reserves: BTreeMap<H160, StableTokenState>,
    pub amplification_parameter: AmplificationParameter,
    pub version: StablePoolVersion,
}

impl StablePool {
    pub fn new_unpaused(pool_id: H160, stable_state: stable::PoolState) -> Self {
        StablePool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id, // V3 pools are contract addresses
                swap_fee: stable_state.swap_fee,
                paused: false,
            },
            reserves: stable_state.tokens.into_iter().collect(),
            amplification_parameter: stable_state.amplification_parameter,
            version: stable_state.version,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StableSurgePool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, StableTokenState>,
    pub amplification_parameter: AmplificationParameter,
    pub version: StablePoolVersion,
    // StableSurge hook parameters
    pub surge_threshold_percentage: Bfp,
    pub max_surge_fee_percentage: Bfp,
}

impl StableSurgePool {
    pub fn new_unpaused(pool_id: H160, state: stable_surge::PoolState) -> Self {
        StableSurgePool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id, // V3 pools are contract addresses
                swap_fee: state.swap_fee,
                paused: false,
            },
            reserves: state.tokens.into_iter().collect(),
            amplification_parameter: state.amplification_parameter,
            version: state.version,
            surge_threshold_percentage: state.surge_threshold_percentage,
            max_surge_fee_percentage: state.max_surge_fee_percentage,
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

#[derive(Clone, Debug)]
pub struct ReClammPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, TokenState>,
    pub version: reclamm::Version,
    pub last_virtual_balances: Vec<U256>,
    pub daily_price_shift_base: Bfp,
    pub last_timestamp: u64,
    pub centeredness_margin: Bfp,
    pub start_fourth_root_price_ratio: Bfp,
    pub end_fourth_root_price_ratio: Bfp,
    pub price_ratio_update_start_time: u64,
    pub price_ratio_update_end_time: u64,
}

impl ReClammPool {
    pub fn new_unpaused(pool_id: H160, reclamm_state: reclamm::PoolState) -> Self {
        ReClammPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id,
                swap_fee: reclamm_state.swap_fee,
                paused: false,
            },
            reserves: reclamm_state.tokens.into_iter().collect(),
            version: reclamm_state.version,
            last_virtual_balances: reclamm_state.last_virtual_balances,
            daily_price_shift_base: reclamm_state.daily_price_shift_base,
            last_timestamp: reclamm_state.last_timestamp,
            centeredness_margin: reclamm_state.centeredness_margin,
            start_fourth_root_price_ratio: reclamm_state.start_fourth_root_price_ratio,
            end_fourth_root_price_ratio: reclamm_state.end_fourth_root_price_ratio,
            price_ratio_update_start_time: reclamm_state.price_ratio_update_start_time,
            price_ratio_update_end_time: reclamm_state.price_ratio_update_end_time,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QuantAmmPool {
    pub common: CommonPoolState,
    pub reserves: BTreeMap<H160, QuantAmmTokenState>,
    pub version: QuantAmmPoolVersion,
    // QuantAMM-specific static data
    pub max_trade_size_ratio: Bfp,
    // QuantAMM-specific dynamic data
    pub first_four_weights_and_multipliers: Vec<I256>,
    pub second_four_weights_and_multipliers: Vec<I256>,
    pub last_update_time: u64,
    pub last_interop_time: u64,
    pub current_timestamp: u64,
}

impl QuantAmmPool {
    pub fn new_unpaused(pool_id: H160, quantamm_state: quantamm::PoolState) -> Self {
        QuantAmmPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id,
                swap_fee: quantamm_state.swap_fee,
                paused: false,
            },
            reserves: quantamm_state.tokens.into_iter().collect(),
            version: quantamm_state.version,
            max_trade_size_ratio: quantamm_state.max_trade_size_ratio,
            first_four_weights_and_multipliers: quantamm_state.first_four_weights_and_multipliers,
            second_four_weights_and_multipliers: quantamm_state.second_four_weights_and_multipliers,
            last_update_time: quantamm_state.last_update_time,
            last_interop_time: quantamm_state.last_interop_time,
            current_timestamp: quantamm_state.current_timestamp, // Use actual block timestamp
        }
    }
}

impl Gyro2CLPPool {
    pub fn new_unpaused(pool_id: H160, gyro_2clp_state: gyro_2clp::PoolState) -> Self {
        Gyro2CLPPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id,
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

impl GyroEPool {
    pub fn new_unpaused(pool_id: H160, gyro_e_state: gyro_e::PoolState) -> Self {
        GyroEPool {
            common: CommonPoolState {
                id: pool_id,
                address: pool_id,
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

#[derive(Default)]
pub struct FetchedBalancerPools {
    pub stable_pools: Vec<StablePool>,
    pub stable_surge_pools: Vec<StableSurgePool>,
    pub weighted_pools: Vec<WeightedPool>,
    pub gyro_2clp_pools: Vec<Gyro2CLPPool>,
    pub gyro_e_pools: Vec<GyroEPool>,
    pub reclamm_pools: Vec<ReClammPool>,
    pub quantamm_pools: Vec<QuantAmmPool>,
}

impl FetchedBalancerPools {
    pub fn relevant_tokens(&self) -> HashSet<H160> {
        let mut tokens = HashSet::new();
        tokens.extend(
            self.weighted_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.stable_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.stable_surge_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.gyro_2clp_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.gyro_e_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.reclamm_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens.extend(
            self.quantamm_pools
                .iter()
                .flat_map(|pool| pool.reserves.keys().copied()),
        );
        tokens
    }
}

#[mockall::automock]
#[async_trait::async_trait]
pub trait BalancerV3PoolFetching: Send + Sync {
    async fn fetch(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<FetchedBalancerPools>;
}

pub struct BalancerPoolFetcher {
    fetcher: Arc<dyn InternalPoolFetching>,
    // We observed some balancer pools being problematic because their token balance becomes out of
    // sync leading to simulation failures.
    pool_id_deny_list: Vec<H160>,
}

/// An enum containing all supported Balancer V3 factory types.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, ValueEnum)]
#[clap(rename_all = "verbatim")]
pub enum BalancerFactoryKind {
    Weighted,
    Stable,
    StableV2,
    StableSurge,
    StableSurgeV2,
    Gyro2CLP,
    GyroE,
    ReClamm,
    QuantAmm,
}

impl BalancerFactoryKind {
    /// Returns a vector with supported factories for the specified chain ID.
    pub fn for_chain(chain_id: u64) -> Vec<Self> {
        match chain_id {
            // Mainnet
            1 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
                Self::QuantAmm,
            ],
            // Gnosis
            100 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
            ],
            // Arbitrum
            42161 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
                Self::QuantAmm,
            ],
            // Optimism
            10 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
            ],
            // Base
            8453 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
                Self::QuantAmm,
            ],
            // Sepolia
            11155111 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
                Self::QuantAmm,
            ],
            // Avalanche
            43114 => vec![
                Self::Weighted,
                Self::Stable,
                Self::StableV2,
                Self::StableSurge,
                Self::StableSurgeV2,
                Self::Gyro2CLP,
                Self::GyroE,
                Self::ReClamm,
            ],
            _ => Default::default(),
        }
    }
}

/// All balancer V3 related contracts that we expect to exist.
pub struct BalancerContracts {
    pub vault: BalancerV3Vault,
    pub batch_router: BalancerV3BatchRouter::Instance,
    pub factories: Vec<(BalancerFactoryKind, DynInstance)>,
}

impl BalancerContracts {
    pub async fn try_new(web3: &Web3, factory_kinds: Vec<BalancerFactoryKind>) -> Result<Self> {
        let web3 = ethrpc::instrumented::instrument_with_label(web3, "balancerV3".into());
        let vault = BalancerV3Vault::deployed(&web3)
            .await
            .context("Cannot retrieve balancer V3 vault")?;
        let batch_router = BalancerV3BatchRouter::Instance::deployed(&web3.alloy)
            .await
            .context("Cannot retrieve balancer V3 batch router")?;

        macro_rules! instance {
            ($factory:ident) => {{
                $factory::deployed(&web3)
                    .await
                    .context(format!(
                        "Cannot retrieve Balancer V3 factory {}",
                        stringify!($factory)
                    ))?
                    .raw_instance()
                    .clone()
            }};
        }

        let mut factories = Vec::new();
        for factory_kind in factory_kinds {
            let factory_instance = match factory_kind {
                BalancerFactoryKind::Weighted => instance!(BalancerV3WeightedPoolFactory),
                BalancerFactoryKind::Stable => instance!(BalancerV3StablePoolFactory),
                BalancerFactoryKind::StableV2 => instance!(BalancerV3StablePoolFactoryV2),
                BalancerFactoryKind::StableSurge => instance!(BalancerV3StableSurgePoolFactory),
                BalancerFactoryKind::StableSurgeV2 => instance!(BalancerV3StableSurgePoolFactoryV2),
                BalancerFactoryKind::Gyro2CLP => instance!(BalancerV3Gyro2CLPPoolFactory),
                BalancerFactoryKind::GyroE => instance!(BalancerV3GyroECLPPoolFactory),
                BalancerFactoryKind::ReClamm => instance!(BalancerV3ReClammPoolFactoryV2),
                BalancerFactoryKind::QuantAmm => instance!(BalancerV3QuantAMMWeightedPoolFactory),
            };
            factories.push((factory_kind, factory_instance));
        }

        Ok(BalancerContracts {
            vault,
            batch_router,
            factories,
        })
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
        deny_listed_pool_ids: Vec<H160>,
        chain: GqlChain,
    ) -> Result<Self> {
        let pool_initializer = BalancerApiClient::from_subgraph_url(subgraph_url, client, chain)?;
        let web3 = ethrpc::instrumented::instrument_with_label(&web3, "balancerV3".into());
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
impl BalancerV3PoolFetching for BalancerPoolFetcher {
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
                    PoolKind::StableSurge(state) => fetched_pools
                        .stable_surge_pools
                        .push(StableSurgePool::new_unpaused(pool.id, state)),
                    PoolKind::Gyro2CLP(state) => fetched_pools
                        .gyro_2clp_pools
                        .push(Gyro2CLPPool::new_unpaused(pool.id, state)),
                    PoolKind::GyroE(state) => fetched_pools
                        .gyro_e_pools
                        .push(GyroEPool::new_unpaused(pool.id, *state)),
                    PoolKind::ReClamm(state) => fetched_pools
                        .reclamm_pools
                        .push(ReClammPool::new_unpaused(pool.id, state)),
                    PoolKind::QuantAmm(state) => fetched_pools
                        .quantamm_pools
                        .push(QuantAmmPool::new_unpaused(pool.id, state)),
                }
                fetched_pools
            },
        );

        Ok(fetched_pools)
    }
}

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
            create_internal_pool_fetcher(
                contracts.vault.clone(),
                $factory::with_deployment_info(
                    &$instance.web3(),
                    $instance.address(),
                    $instance.deployment_information(),
                ),
                block_retriever.clone(),
                token_infos.clone(),
                $instance,
                registered_pools_by_factory
                    .remove(&$instance.address())
                    .unwrap_or_else(|| RegisteredPools::empty(fetched_block_number)),
                fetched_block_hash,
            )?
        }};
    }

    let mut fetchers = Vec::new();
    for (kind, instance) in &contracts.factories {
        let registry = match kind {
            BalancerFactoryKind::Weighted => {
                registry!(BalancerV3WeightedPoolFactory, instance)
            }
            BalancerFactoryKind::Stable => {
                registry!(BalancerV3StablePoolFactory, instance)
            }
            BalancerFactoryKind::StableV2 => {
                registry!(BalancerV3StablePoolFactoryV2, instance)
            }
            BalancerFactoryKind::StableSurge => {
                registry!(BalancerV3StableSurgePoolFactory, instance)
            }
            BalancerFactoryKind::StableSurgeV2 => {
                registry!(BalancerV3StableSurgePoolFactoryV2, instance)
            }
            BalancerFactoryKind::Gyro2CLP => {
                registry!(BalancerV3Gyro2CLPPoolFactory, instance)
            }
            BalancerFactoryKind::GyroE => {
                registry!(BalancerV3GyroECLPPoolFactory, instance)
            }
            BalancerFactoryKind::ReClamm => {
                registry!(BalancerV3ReClammPoolFactoryV2, instance)
            }
            BalancerFactoryKind::QuantAmm => {
                registry!(BalancerV3QuantAMMWeightedPoolFactory, instance)
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
            "found pools that don't correspond to any known Balancer V3 pool factory",
        );
    }

    Ok(Aggregate::new(fetchers))
}

fn create_internal_pool_fetcher<Factory>(
    vault: BalancerV3Vault,
    factory: Factory,
    block_retriever: Arc<dyn BlockRetrieving>,
    token_infos: Arc<dyn TokenInfoFetching>,
    factory_instance: &Instance<Web3Transport>,
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
        Arc::new(PoolInfoFetcher::new(vault, factory, token_infos)),
        factory_instance,
        initial_pools,
        start_sync_at_block,
    )))
}
