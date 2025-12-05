use {
    crate::{
        domain::{eth, solver},
        infra::contracts,
        util::serialize,
    },
    chain::Chain,
    ethereum_types::H160,
    serde::Deserialize,
    serde_with::serde_as,
    shared::price_estimation::gas::SETTLEMENT_OVERHEAD,
    std::{fmt::Debug, path::Path},
    tokio::fs,
    url::Url,
};

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Config {
    /// Optional chain ID. This is used to automatically determine the address
    /// of the WETH contract.
    chain_id: Option<Chain>,

    /// Optional WETH contract address. This can be used to specify a manual
    /// value **instead** of using the canonical WETH contract for the
    /// configured chain.
    weth: Option<H160>,

    /// List of base tokens to use when path finding. This defines the tokens
    /// that can appear as intermediate "hops" within a trading route. Note that
    /// WETH is always considered as a base token.
    base_tokens: Vec<eth::H160>,

    /// The maximum number of hops to consider when finding the optimal trading
    /// path.
    max_hops: usize,

    /// The maximum number of pieces to divide partially fillable limit orders
    /// when trying to solve it against baseline liquidity.
    max_partial_attempts: usize,

    /// Units of gas that get added to the gas estimate for executing a
    /// computed trade route to arrive at a gas estimate for a whole settlement.
    #[serde(default = "default_gas_offset")]
    solution_gas_offset: i64,

    /// The amount of the native token to use to estimate native price of a
    /// token
    #[serde_as(as = "serialize::U256")]
    native_token_price_estimation_amount: eth::U256,

    /// If this is configured the solver will also use the Uniswap V3 liquidity
    /// sources that rely on RPC request.
    uni_v3_node_url: Option<Url>,

    /// Optional RPC endpoint used for ERC4626 preview_* quoting in baseline
    /// routing. If unset, we fall back to `uni_v3_node_url` if present. If
    /// both are unset, ERC4626 baseline routing is disabled.
    erc4626_node_url: Option<Url>,

    /// Configuration for independent liquidity fetching
    liquidity: Option<LiquidityConfig>,

    /// Optional directory path to save auction and solution JSON files
    auction_save_directory: Option<String>,

    /// Balancer V2 Vault address for solution verification
    vault_address: Option<H160>,

    /// Balancer V3 Batch Router address for solution verification
    batch_router_address: Option<H160>,

    /// Node URL for solution verification
    node_url: Option<Url>,

    /// Feature toggles for logging and verification artifacts
    #[serde(default)]
    logging: LoggingSection,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case", default)]
struct LoggingSection {
    auction_files: bool,
    competition: bool,
    swap_logs: bool,
    swap_log_verification: bool,
    solution_verification: bool,
    enhanced_solutions: bool,
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            auction_files: true,
            competition: true,
            swap_logs: true,
            swap_log_verification: true,
            solution_verification: true,
            enhanced_solutions: true,
        }
    }
}

/// Configuration for the liquidity client
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct LiquidityConfig {
    /// URL of the liquidity-driver instance
    pub driver_url: String,

    /// Request timeout in milliseconds
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Protocols to fetch liquidity from
    #[serde(default = "default_protocols")]
    pub protocols: Vec<String>,
}

fn default_timeout_ms() -> u64 {
    5000
}

fn default_protocols() -> Vec<String> {
    vec!["balancer_v2".to_string(), "uniswap_v2".to_string()]
}

/// Load the driver configuration from a TOML file.
///
/// # Panics
///
/// This method panics if the config is invalid or on I/O errors.
pub async fn load(path: &Path) -> solver::Config {
    let data = fs::read_to_string(path)
        .await
        .unwrap_or_else(|e| panic!("I/O error while reading {path:?}: {e:?}"));
    // Not printing detailed error because it could potentially leak secrets.
    let config = unwrap_or_log(toml::de::from_str::<Config>(&data), &path);
    let weth = match (config.chain_id, config.weth) {
        (Some(chain_id), None) => contracts::Contracts::for_chain(chain_id).weth,
        (None, Some(weth)) => eth::WethAddress(weth),
        (Some(_), Some(_)) => panic!(
            "invalid configuration: cannot specify both `chain-id` and `weth` configuration \
             options",
        ),
        (None, None) => panic!(
            "invalid configuration: must specify either `chain-id` or `weth` configuration options",
        ),
    };

    // Build base tokens, including ERC4626 vault addresses if configured.
    // This enables routing through ERC4626 vaults as intermediate hops.
    let mut base_tokens: Vec<eth::TokenAddress> = config
        .base_tokens
        .into_iter()
        .map(eth::TokenAddress)
        .collect();

    // Add ERC4626 vault addresses to base tokens for routing
    if let Some(chain_id) = config.chain_id {
        let erc4626_vaults = get_erc4626_vault_addresses(chain_id);
        base_tokens.extend(erc4626_vaults.into_iter().map(eth::TokenAddress));
    }

    solver::Config {
        chain_id: config.chain_id.map(|c| c as u64).unwrap_or(1),
        weth,
        base_tokens,
        max_hops: config.max_hops,
        max_partial_attempts: config.max_partial_attempts,
        solution_gas_offset: config.solution_gas_offset.into(),
        native_token_price_estimation_amount: config.native_token_price_estimation_amount,
        uni_v3_node_url: config.uni_v3_node_url,
        erc4626_node_url: config.erc4626_node_url,
        liquidity_client_config: config.liquidity,
        auction_save_directory: config.auction_save_directory.map(std::path::PathBuf::from),
        vault_address: config.vault_address.map(eth::Address),
        batch_router_address: config.batch_router_address.map(eth::Address),
        node_url: config.node_url,
        logging: solver::LoggingConfig {
            auction_files: config.logging.auction_files,
            competition: config.logging.competition,
            swap_logs: config.logging.swap_logs,
            swap_log_verification: config.logging.swap_log_verification,
            solution_verification: config.logging.solution_verification,
            enhanced_solutions: config.logging.enhanced_solutions,
        },
    }
}

/// Returns the ERC4626 vault addresses from the config file if enabled.
/// These are added to base tokens to enable routing through vaults.
fn get_erc4626_vault_addresses(chain: Chain) -> Vec<eth::H160> {
    let config_dir = match chain {
        Chain::ArbitrumOne => "arbitrum",
        Chain::Mainnet => "mainnet",
        Chain::Goerli => "goerli",
        Chain::Sepolia => "sepolia",
        Chain::Gnosis => "gnosis",
        Chain::Base => "base",
        Chain::Bnb => "bnb",
        Chain::Avalanche => "avalanche",
        Chain::Optimism => "optimism",
        Chain::Polygon => "polygon",
        Chain::Hardhat => "hardhat",
        Chain::Lens => "lens",
        Chain::Linea => "linea",
        Chain::Plasma => "plasma",
    };

    let primary_path = format!("configs/{}/erc4626.toml", config_dir);
    let fallback_path = format!("../{}", primary_path);

    let config = shared::sources::erc4626::registry::load_config_from_file(std::path::Path::new(
        &primary_path,
    ))
    .or_else(|_| {
        shared::sources::erc4626::registry::load_config_from_file(std::path::Path::new(
            &fallback_path,
        ))
    });

    match config {
        Ok(cfg) if cfg.enabled => {
            tracing::debug!(
                vault_count = cfg.vaults.len(),
                "Adding ERC4626 vault tokens to base tokens"
            );
            cfg.vaults.into_iter().map(eth::H160::from).collect()
        }
        _ => {
            tracing::debug!("ERC4626 not enabled; no vault tokens added to base tokens");
            vec![]
        }
    }
}

/// Unwraps result or logs a `TOML` parsing error.
fn unwrap_or_log<T, E, P>(result: Result<T, E>, path: &P) -> T
where
    E: Debug,
    P: Debug,
{
    result.unwrap_or_else(|err| {
        if std::env::var("TOML_TRACE_ERROR").is_ok_and(|v| v == "1") {
            panic!("failed to parse TOML config at {path:?}: {err:#?}")
        } else {
            panic!(
                "failed to parse TOML config at: {path:?}. Set TOML_TRACE_ERROR=1 to print \
                 parsing error but this may leak secrets."
            )
        }
    })
}

/// Returns minimum gas used for settling a single order.
/// (not accounting for the cost of additional interactions)
fn default_gas_offset() -> i64 {
    SETTLEMENT_OVERHEAD.try_into().unwrap()
}
