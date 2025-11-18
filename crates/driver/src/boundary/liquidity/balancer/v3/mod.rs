use {
    crate::{
        boundary,
        domain::{
            eth,
            liquidity::{self, balancer},
        },
        infra::{self, blockchain::Ethereum},
    },
    anyhow::{Context, Result},
    chain::Chain,
    contracts::{
        BalancerV3Gyro2CLPPoolFactory,
        BalancerV3GyroECLPPoolFactory,
        BalancerV3QuantAMMWeightedPoolFactory,
        BalancerV3StablePoolFactory,
        BalancerV3StablePoolFactoryV2,
        BalancerV3StableSurgePoolFactory,
        BalancerV3StableSurgePoolFactoryV2,
        BalancerV3Vault,
        BalancerV3WeightedPoolFactory,
        alloy::GPv2Settlement,
    },
    ethrpc::{
        alloy::conversions::IntoAlloy,
        block_stream::{BlockRetrieving, CurrentBlockWatcher},
    },
    shared::{
        http_solver::model::TokenAmount,
        sources::balancer_v3::{
            BalancerFactoryKind,
            BalancerPoolFetcher,
            GqlChain,
            pool_fetching::BalancerContracts,
        },
        token_info::{CachedTokenInfoFetcher, TokenInfoFetcher},
    },
    solver::{
        interactions::allowances::Allowances,
        liquidity::{balancer_v3, balancer_v3::BalancerV3Liquidity},
        liquidity_collector::{BackgroundInitLiquiditySource, LiquidityCollecting},
    },
    std::sync::Arc,
};

pub mod gyro_2clp;
pub mod gyro_e;
pub mod quantamm;
pub mod reclamm;
pub mod stable;
pub mod stable_surge;
pub mod weighted;

/// Maps a Chain to the corresponding GqlChain for Balancer V3 API.
fn chain_to_gql_chain(chain: &Chain) -> GqlChain {
    match chain {
        Chain::Mainnet => GqlChain::MAINNET,
        Chain::Goerli => GqlChain::SEPOLIA, // Goerli is deprecated, use Sepolia
        Chain::Sepolia => GqlChain::SEPOLIA,
        Chain::Gnosis => GqlChain::GNOSIS,
        Chain::Polygon => GqlChain::POLYGON,
        Chain::ArbitrumOne => GqlChain::ARBITRUM,
        Chain::Optimism => GqlChain::OPTIMISM,
        Chain::Base => GqlChain::BASE,
        Chain::Bnb => GqlChain::BSC,
        Chain::Avalanche => GqlChain::AVALANCHE,
        Chain::Hardhat => GqlChain::MAINNET, // Hardhat is a local testnet, default to mainnet
        Chain::Lens => GqlChain::LENS,
        Chain::Linea => GqlChain::LINEA,
        Chain::Plasma => GqlChain::PLASMA,
    }
}

struct Pool {
    batch_router: eth::ContractAddress,
    id: balancer::v3::Id,
}

fn to_interaction(
    pool: &Pool,
    input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> eth::Interaction {
    let handler = balancer_v3::SettlementHandler::new(
        pool.id.into(),
        // Note that this code assumes `receiver == sender`. This assumption is
        // also baked into the Balancer V3 logic in the `shared` crate, so to
        // change this assumption, we would need to change it there as well.
        GPv2Settlement::Instance::new(receiver.0.into_alloy(), ethrpc::mock::web3().alloy.clone()),
        contracts::alloy::BalancerV3BatchRouter::Instance::new(
            pool.batch_router.0.into_alloy(),
            ethrpc::mock::web3().alloy,
        ),
        Allowances::empty(receiver.0),
    );

    let interaction = handler.swap(
        TokenAmount::new(input.0.token.into(), input.0.amount),
        TokenAmount::new(output.0.token.into(), output.0.amount),
    );

    let (target, value, call_data) = interaction.encode_swap();

    eth::Interaction {
        target: target.into(),
        value: value.into(),
        call_data: call_data.0.into(),
    }
}

pub fn collector(
    eth: &Ethereum,
    block_stream: CurrentBlockWatcher,
    block_retriever: Arc<dyn BlockRetrieving>,
    config: &infra::liquidity::config::BalancerV3,
) -> Box<dyn LiquidityCollecting> {
    let eth = Arc::new(eth.with_metric_label("balancerV3".into()));
    let reinit_interval = config.reinit_interval;
    let config = Arc::new(config.clone());
    let init = move || {
        let eth = eth.clone();
        let block_stream = block_stream.clone();
        let block_retriever = block_retriever.clone();
        let config = config.clone();
        async move { init_liquidity(&eth, &block_stream, block_retriever.clone(), &config).await }
    };
    const TEN_MINUTES: std::time::Duration = std::time::Duration::from_secs(10 * 60);
    Box::new(BackgroundInitLiquiditySource::new(
        "balancer-v3",
        init,
        TEN_MINUTES,
        reinit_interval,
    )) as Box<_>
}

async fn init_liquidity(
    eth: &Ethereum,
    block_stream: &CurrentBlockWatcher,
    block_retriever: Arc<dyn BlockRetrieving>,
    config: &infra::liquidity::config::BalancerV3,
) -> Result<impl LiquidityCollecting + use<>> {
    let web3 = eth.web3().clone();

    // Create Balancer V3 contracts configuration
    let contracts = BalancerContracts {
        vault: BalancerV3Vault::at(&web3, config.vault.into()),
        batch_router: contracts::alloy::BalancerV3BatchRouter::Instance::new(
            config.batch_router.0.into_alloy(),
            web3.alloy.clone(),
        ),
        factories: [
            config
                .weighted
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::Weighted,
                        BalancerV3WeightedPoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .stable
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::Stable,
                        BalancerV3StablePoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .stable_v2
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::StableV2,
                        BalancerV3StablePoolFactoryV2::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .stable_surge
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::StableSurge,
                        BalancerV3StableSurgePoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .stable_surge_v2
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::StableSurgeV2,
                        BalancerV3StableSurgePoolFactoryV2::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .gyro_e
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::GyroE,
                        BalancerV3GyroECLPPoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .gyro_2clp
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::Gyro2CLP,
                        BalancerV3Gyro2CLPPoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .reclamm
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::ReClamm,
                        contracts::BalancerV3ReClammPoolFactoryV2::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
            config
                .quantamm
                .iter()
                .map(|&factory| {
                    (
                        BalancerFactoryKind::QuantAmm,
                        BalancerV3QuantAMMWeightedPoolFactory::at(&web3, factory.into())
                            .raw_instance()
                            .clone(),
                    )
                })
                .collect::<Vec<_>>(),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    let token_info_fetcher = Arc::new(CachedTokenInfoFetcher::new(Arc::new(TokenInfoFetcher {
        web3: web3.clone(),
    })));

    let balancer_pool_fetcher = Arc::new(
        BalancerPoolFetcher::new(
            &config.graph_url,
            block_retriever.clone(),
            token_info_fetcher.clone(),
            boundary::liquidity::cache_config(),
            block_stream.clone(),
            boundary::liquidity::http_client(),
            web3.clone(),
            &contracts,
            config.pool_deny_list.clone(),
            chain_to_gql_chain(&eth.chain()),
        )
        .await
        .context("failed to create Balancer V3 pool fetcher")?,
    );

    Ok(BalancerV3Liquidity::new(
        web3,
        balancer_pool_fetcher,
        eth.contracts().settlement().clone(),
        contracts.batch_router,
    ))
}
