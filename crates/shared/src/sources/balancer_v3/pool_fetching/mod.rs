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
            weighted,
        },
        swap::fixed_point::Bfp,
    },
    crate::{
        ethrpc::{Web3, Web3Transport},
        recent_block_cache::{Block, CacheConfig},
        token_info::TokenInfoFetching,
    },
    anyhow::{Context, Result},
    clap::ValueEnum,
    contracts::{
        BalancerV3WeightedPoolFactory,
        BalancerV3Vault,
    },
    ethcontract::{BlockId, H160, Instance, dyns::DynInstance},
    ethrpc::block_stream::{BlockRetrieving, CurrentBlockWatcher},
    model::TokenPair,
    reqwest::{Client, Url},
    std::{
        collections::{BTreeMap, HashMap, HashSet},
        sync::Arc,
    },
};
pub use {
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

#[derive(Default)]
pub struct FetchedBalancerPools {
    pub weighted_pools: Vec<WeightedPool>,
}

impl FetchedBalancerPools {
    pub fn relevant_tokens(&self) -> HashSet<H160> {
        let mut tokens = HashSet::new();
        tokens.extend(
            self.weighted_pools
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
    // We observed some balancer pools being problematic because their token balance becomes out of sync leading to simulation
    // failures.
    pool_id_deny_list: Vec<H160>,
}

/// An enum containing all supported Balancer V3 factory types.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, ValueEnum)]
#[clap(rename_all = "verbatim")]
pub enum BalancerFactoryKind {
    Weighted,
}

impl BalancerFactoryKind {
    /// Returns a vector with supported factories for the specified chain ID.
    pub fn for_chain(chain_id: u64) -> Vec<Self> {
        match chain_id {
            1 => vec![Self::Weighted],
            5 => vec![Self::Weighted],
            100 => vec![Self::Weighted],
            11155111 => vec![Self::Weighted],
            _ => Default::default(),
        }
    }
}

/// All balancer V3 related contracts that we expect to exist.
pub struct BalancerContracts {
    pub vault: BalancerV3Vault,
    pub factories: Vec<(BalancerFactoryKind, DynInstance)>,
}

impl BalancerContracts {
    pub async fn try_new(web3: &Web3, factory_kinds: Vec<BalancerFactoryKind>) -> Result<Self> {
        let web3 = ethrpc::instrumented::instrument_with_label(web3, "balancerV3".into());
        let vault = BalancerV3Vault::deployed(&web3)
            .await
            .context("Cannot retrieve balancer V3 vault")?;

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
            };
            factories.push((factory_kind, factory_instance));
        }

        Ok(BalancerContracts { vault, factories })
    }
}

impl BalancerPoolFetcher {
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
        let api_client = BalancerApiClient::from_subgraph_url(subgraph_url, client, chain)?;
        let registered_pools = api_client.get_registered_pools().await?;
        let aggregate = create_aggregate_pool_fetcher(
            web3,
            PoolInitializing::new(registered_pools),
            block_retriever,
            token_infos,
            contracts,
        )
        .await?;
        let fetcher = Cache::new(aggregate, config, block_stream)?;

        Ok(Self {
            fetcher: Arc::new(fetcher),
            pool_id_deny_list: deny_listed_pool_ids,
        })
    }

    async fn fetch_pools(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<Vec<Pool>> {
        let pool_ids = self.fetcher.pool_ids_for_token_pairs(token_pairs).await;
        let filtered_pool_ids: HashSet<H160> = pool_ids
            .into_iter()
            .filter(|pool_id| !self.pool_id_deny_list.contains(pool_id))
            .collect();
        self.fetcher.pools_by_id(filtered_pool_ids, at_block).await
    }
}

impl BalancerPoolFetching for BalancerPoolFetcher {
    async fn fetch(
        &self,
        token_pairs: HashSet<TokenPair>,
        at_block: Block,
    ) -> Result<FetchedBalancerPools> {
        let pools = self.fetch_pools(token_pairs, at_block).await?;
        let mut fetched_pools = FetchedBalancerPools::default();

        for pool in pools {
            match pool {
                Pool::Weighted(weighted_pool) => {
                    fetched_pools.weighted_pools.push(WeightedPool::new_unpaused(
                        weighted_pool.common.id,
                        weighted_pool,
                    ));
                }
            }
        }

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
    let mut fetchers = Vec::new();

    for (factory_kind, factory_instance) in &contracts.factories {
        let registered_pools = pool_initializer
            .get_pools_for_factory(*factory_kind)
            .await?;

        let fetcher = match factory_kind {
            BalancerFactoryKind::Weighted => {
                create_internal_pool_fetcher(
                    contracts.vault.clone(),
                    weighted::BalancerV3WeightedPoolFactory,
                    block_retriever.clone(),
                    token_infos.clone(),
                    factory_instance,
                    registered_pools,
                    registered_pools.fetched_block_number,
                )
                .await?
            }
        };

        fetchers.push(fetcher);
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
    fetched_block_hash: u64,
) -> Result<Box<dyn InternalPoolFetching>>
where
    Factory: FactoryIndexing,
{
    let pool_info_fetcher = Arc::new(PoolInfoFetcher::new(
        vault,
        factory_instance.clone(),
        token_infos,
    ));

    let storage = pool_storage::PoolStorage::new(
        registered_pools.pools.into_iter().map(|pool_data| {
            Factory::PoolInfo::from_graph_data(pool_data, fetched_block_hash)
        }).collect(),
        pool_info_fetcher,
    );

    let registry = Registry::new(storage, block_retriever);
    Ok(Box::new(registry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_extract_address_from_pool_id() {
        let pool_id = H160([1; 20]);
        let address = pool_address_from_id(pool_id);
        assert_eq!(address, pool_id);
    }
} 