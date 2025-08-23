//! Module containing the Balancer V3 API client used for retrieving Balancer V3
//! pools.
//!
//! The pools retrieved from this client are used to prime the graph event store
//! to reduce start-up time. We do not use this in general for retrieving pools
//! as to:
//! - not rely on external services
//! - ensure that we are using the latest up-to-date pool data by using events
//!   from the node

const QUERY_PAGE_SIZE: usize = 100;

/// Custom deserializer that converts empty strings to None for optional SBfp
/// fields. This ensures consistency with V2 and provides robust handling of any
/// potential empty string issues in the V3 API response. Also handles automatic
/// precision detection for high-precision decimal values.
fn deserialize_optional_sbfp<'de, D>(deserializer: D) -> Result<Option<SBfp>, D::Error>
where
    D: Deserializer<'de>,
{
    // First try to deserialize as Option<String> to handle both null and string
    // values
    let opt_s = Option::<String>::deserialize(deserializer)?;

    match opt_s {
        // Handle null values
        None => Ok(None),
        // Handle string values
        Some(s) => {
            // Convert empty strings to None (like null values)
            if s.is_empty() {
                return Ok(None);
            }

            // Parse valid decimal strings with automatic precision detection
            // (same logic as the original SBfp::Deserialize implementation)
            use crate::sources::balancer_v3::swap::signed_fixed_point::FixedPointPrecision;

            let precision = if s.contains('.')
                && s.split('.').nth(1).map_or(0, |decimals| decimals.len()) > 30
            {
                FixedPointPrecision::Extended38
            } else {
                FixedPointPrecision::Standard18
            };

            SBfp::from_str_with_precision(&s, precision)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
}

use {
    super::swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    crate::subgraph::SubgraphClient,
    anyhow::{Context, Result},
    ethcontract::H160,
    reqwest::{Client, Url},
    serde::{Deserialize, Deserializer, Serialize},
    serde_json::json,
    serde_with::{DisplayFromStr, serde_as},
    std::collections::HashMap,
};

/// Balancer V3 API client for fetching pool data.
pub struct BalancerApiClient {
    client: SubgraphClient,
    chain: GqlChain,
}

/// Supported chains in Balancer V3 API.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
pub enum GqlChain {
    MAINNET,
    GNOSIS,
    ARBITRUM,
    POLYGON,
    BASE,
    AVALANCHE,
    OPTIMISM,
    BSC,
    FANTOM,
    SEPOLIA,
    LENS,
}

impl BalancerApiClient {
    /// Creates a new Balancer V3 API client.
    pub fn from_subgraph_url(subgraph_url: &Url, client: Client, chain: GqlChain) -> Result<Self> {
        let subgraph_client = SubgraphClient::try_new(subgraph_url.clone(), client, None)?;
        Ok(Self {
            client: subgraph_client,
            chain,
        })
    }

    /// Retrieves all registered pools for the configured chain.
    pub async fn get_registered_pools(&self) -> Result<RegisteredPools> {
        use self::pools_query::*;

        let mut pools = Vec::new();
        let mut skip = 0;

        // Use offset-based pagination with Balancer V3 API
        loop {
            let page = self
                .client
                .query::<Data>(
                    QUERY,
                    Some(json_map! {
                        "first" => QUERY_PAGE_SIZE,
                        "skip" => skip,
                        "orderBy" => "totalLiquidity",
                        "orderDirection" => "desc",
                        "where" => json!({
                            "includeHooks": "STABLE_SURGE",
                            "chainIn": [self.chain],
                             "poolTypeIn": ["WEIGHTED", "STABLE", "GYROE", "RECLAMM", "QUANT_AMM_WEIGHTED", "GYRO"],
                            "protocolVersionIn": [3] // V3 protocol
                        }),
                    }),
                )
                .await?
                .aggregator_pools;

            let no_more_pages = page.len() != QUERY_PAGE_SIZE;
            pools.extend(page);

            if no_more_pages {
                break;
            }

            skip += QUERY_PAGE_SIZE;
        }

        Ok(RegisteredPools {
            fetched_block_number: 0, // Balancer V3 API doesn't support historical queries
            pools,
        })
    }
}

/// Result of the registered pool query.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct RegisteredPools {
    /// The block number that the data was fetched. Set to 0 for Balancer V3 API
    /// since it doesn't support historical queries.
    pub fetched_block_number: u64,
    /// The registered Pools
    pub pools: Vec<PoolData>,
}

impl RegisteredPools {
    /// Creates an empty collection of registered pools for the specified block
    /// number.
    pub fn empty(fetched_block_number: u64) -> Self {
        Self {
            fetched_block_number,
            ..Default::default()
        }
    }

    /// Groups registered pools by factory addresses.
    pub fn group_by_factory(self) -> HashMap<H160, RegisteredPools> {
        let fetched_block_number = self.fetched_block_number;
        self.pools
            .into_iter()
            .fold(HashMap::new(), |mut grouped, pool| {
                grouped
                    .entry(pool.factory)
                    .or_insert(RegisteredPools {
                        fetched_block_number,
                        ..Default::default()
                    })
                    .pools
                    .push(pool);
                grouped
            })
    }
}

/// Pool data from the Balancer V3 API.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PoolData {
    pub id: String, // V3 uses 20-byte pool addresses as IDs
    pub address: H160,
    #[serde(rename = "type")]
    pub pool_type: String,
    pub protocol_version: u32,
    pub factory: H160,
    pub chain: GqlChain,
    pub pool_tokens: Vec<Token>,
    pub dynamic_data: DynamicData,
    pub create_time: u64,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub alpha: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub beta: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub c: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub s: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub lambda: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub tau_alpha_x: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub tau_alpha_y: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub tau_beta_x: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub tau_beta_y: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub u: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub v: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub w: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub z: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub d_sq: Option<SBfp>,
    /// Gyro 2-CLP-specific parameters
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub sqrt_alpha: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub sqrt_beta: Option<SBfp>,
    /// QuantAMM-specific parameters
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub max_trade_size_ratio: Option<Bfp>,
    /// Hook configuration for the pool (matches GraphQL nested structure)
    #[serde(default)]
    pub hook: Option<HookConfig>,
}

/// Hook configuration that matches the GraphQL response structure.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookConfig {
    pub address: H160,
    #[serde(default)]
    pub params: Option<HookParams>,
}

/// Hook parameters for different hook types.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookParams {
    /// StableSurge hook parameters
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub max_surge_fee_percentage: Option<Bfp>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub surge_threshold_percentage: Option<Bfp>,
}

/// Dynamic data for pools from Balancer V3 API.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DynamicData {
    pub swap_enabled: bool,
}

/// Token data for pools.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Token {
    pub address: H160,
    pub decimals: u8,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub weight: Option<Bfp>,
    #[serde(rename = "priceRateProvider")]
    pub price_rate_provider: Option<H160>,
}

/// Supported pool kinds for V3.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash)]
pub enum PoolType {
    Weighted,         // BalancerV3WeightedPoolFactory
    Stable,           // BalancerV3StablePoolFactory, BalancerV3StablePoolFactoryV2
    GyroE,            // BalancerV3GyroECLPPoolFactory
    Gyro2CLP,         // BalancerV3Gyro2CLPPoolFactory
    ReClamm,          // BalancerV3ReClammPoolFactoryV2
    QuantAmmWeighted, // BalancerV3QuantAMMWeightedPoolFactory
}

impl PoolData {
    /// Converts the API pool type string to our internal enum.
    pub fn pool_type_enum(&self) -> PoolType {
        match self.pool_type.as_str() {
            "WEIGHTED" => PoolType::Weighted,
            "STABLE" => PoolType::Stable,
            "GYROE" => PoolType::GyroE,
            "GYRO" => PoolType::Gyro2CLP,
            "RECLAMM" => PoolType::ReClamm,
            "QUANT_AMM_WEIGHTED" => PoolType::QuantAmmWeighted,
            _ => panic!("Unknown pool type: {}", self.pool_type),
        }
    }

    /// Returns the swap enabled status from dynamic data.
    pub fn swap_enabled(&self) -> bool {
        self.dynamic_data.swap_enabled
    }

    /// Returns the tokens with the correct field mapping.
    pub fn tokens(&self) -> Vec<Token> {
        self.pool_tokens.clone()
    }

    /// Converts the string ID to H160. For V3 pools, this should be a 20-byte
    /// hex string.
    pub fn id_as_h160(&self) -> Result<H160> {
        let id_str = self.id.trim_start_matches("0x");
        if id_str.len() == 40 {
            let id_bytes = hex::decode(id_str).context("Failed to decode pool ID as hex")?;
            Ok(H160::from_slice(&id_bytes))
        } else {
            Err(anyhow::anyhow!(
                "Invalid pool ID length for V3 pool: {} (expected 40 hex chars, got {})",
                self.id,
                id_str.len()
            ))
        }
    }

    /// Returns true if this is a V3 pool (protocol version 3).
    pub fn is_v3_pool(&self) -> bool {
        self.protocol_version == 3
    }
}

mod pools_query {
    use serde::Deserialize;

    pub const QUERY: &str = r#"
        query aggregatorPools(
            $first: Int,
            $skip: Int,
            $orderBy: GqlPoolOrderBy,
            $orderDirection: GqlPoolOrderDirection,
            $where: GqlAggregatorPoolFilter
        ) {
            aggregatorPools(
                first: $first
                skip: $skip
                orderBy: $orderBy
                orderDirection: $orderDirection
                where: $where
            ) {
                id
                address
                type
                protocolVersion
                factory
                chain
                poolTokens {
                    address
                    decimals
                    weight
                    priceRateProvider
                }
                dynamicData {
                    swapEnabled
                }
                createTime
                alpha
                beta
                c
                s
                lambda
                tauAlphaX
                tauAlphaY
                tauBetaX
                tauBetaY
                u
                v
                w
                z
                dSq
                quantAmmWeightedParams {
                    maxTradeSizeRatio
                }
                sqrtAlpha
                sqrtBeta
                hook {
                    address
                    params {
                        ... on StableSurgeHookParams {
                            maxSurgeFeePercentage
                            surgeThresholdPercentage
                        }
                    }
                }
            }
        }
    "#;

    #[derive(Debug, Deserialize)]
    pub struct Data {
        #[serde(rename = "aggregatorPools")]
        pub aggregator_pools: Vec<super::PoolData>,
    }
}

#[cfg(test)]
mod tests {
    use {super::*, ethcontract::H160};

    #[test]
    fn decode_pools_data() {
        let json = r#"{
            "aggregatorPools": [
                {
                    "id": "0x1111111111111111111111111111111111111111",
                    "address": "0x1111111111111111111111111111111111111111",
                    "type": "WEIGHTED",
                    "protocolVersion": 3,
                    "factory": "0x2222222222222222222222222222222222222222",
                    "chain": "MAINNET",
                    "poolTokens": [
                        {
                            "address": "0x3333333333333333333333333333333333333333",
                            "decimals": 18,
                            "weight": "0.5"
                        }
                    ],
                    "dynamicData": {
                        "swapEnabled": true
                    },
                    "createTime": 1234567890,
                    "hook": null
                }
            ]
        }"#;

        let data: pools_query::Data = serde_json::from_str(json).unwrap();
        assert_eq!(data.aggregator_pools.len(), 1);
        let pool = &data.aggregator_pools[0];
        assert_eq!(pool.id, "0x1111111111111111111111111111111111111111");
        assert_eq!(pool.address, H160([0x11; 20]));
        assert_eq!(pool.pool_type_enum(), PoolType::Weighted);
        assert!(pool.swap_enabled());
        assert_eq!(pool.tokens().len(), 1);
        assert_eq!(pool.tokens()[0].address, H160([0x33; 20]));
    }

    #[test]
    fn decode_gyro_pools_with_mixed_null_and_empty_params() {
        use pools_query::*;

        // Test that both null and empty strings are handled correctly in V3
        let mixed_json = json!({
            "aggregatorPools": [
                {
                    "id": "0x1111111111111111111111111111111111111111",
                    "address": "0x1111111111111111111111111111111111111111",
                    "type": "GYROE",
                    "protocolVersion": 3,
                    "factory": "0x2222222222222222222222222222222222222222",
                    "chain": "MAINNET",
                    "poolTokens": [
                        {
                            "address": "0x3333333333333333333333333333333333333333",
                            "decimals": 18,
                            "weight": "0.5"
                        }
                    ],
                    "dynamicData": { "swapEnabled": true },
                    "createTime": 1234567890,
                    // E-CLP parameters with valid values
                    "alpha": "0.7",
                    "beta": "1.3",
                    "c": "0.707106781186547524",
                    "s": "0.707106781186547524",
                    "lambda": "1",
                    "tauAlphaX": "-0.17378533390904767196396190604716688",
                    "tauAlphaY": "0.984783558817936807795784134267279",
                    "tauBetaX": "0.1293391840677680520489165354049038",
                    "tauBetaY": "0.9916004111862217323750267714375956",
                    "u": "0.1515622589884078618346041354467426",
                    "v": "0.9881919850020792689650338303356912",
                    "w": "0.003408426184142462285756984496121705",
                    "z": "-0.022223074920639809932327072642593141",
                    "dSq": "0.9999999999999999988662409334210612",
                    // 2-CLP parameters should be None for E-CLP pools (null in V3)
                    "sqrtAlpha": null,
                    "sqrtBeta": null,
                    // QuantAMM parameter
                    "maxTradeSizeRatio": null,
                    // Hook configuration
                    "hook": null
                }
            ]
        });

        let data: Data = serde_json::from_value(mixed_json).unwrap();
        let pool = &data.aggregator_pools[0];

        // Verify E-CLP parameters parsed correctly
        assert!(pool.alpha.is_some());
        assert!(pool.beta.is_some());
        assert!(pool.c.is_some());
        assert!(pool.s.is_some());
        assert!(pool.lambda.is_some());
        assert!(pool.tau_alpha_x.is_some());
        assert!(pool.tau_alpha_y.is_some());
        assert!(pool.tau_beta_x.is_some());
        assert!(pool.tau_beta_y.is_some());
        assert!(pool.u.is_some());
        assert!(pool.v.is_some());
        assert!(pool.w.is_some());
        assert!(pool.z.is_some());
        assert!(pool.d_sq.is_some());

        // Verify null values converted to None
        assert!(pool.sqrt_alpha.is_none());
        assert!(pool.sqrt_beta.is_none());
        assert!(pool.max_trade_size_ratio.is_none());

        // Verify pool type identification
        assert_eq!(pool.pool_type_enum(), PoolType::GyroE);

        println!("Successfully parsed V3 GyroE pool with null 2-CLP params:");
        println!("  alpha: {:?}", pool.alpha);
        println!("  sqrtAlpha: {:?}", pool.sqrt_alpha);
    }

    #[test]
    fn decode_stable_surge_hook_data() {
        let json = r#"{
            "aggregatorPools": [
                {
                    "id": "0x1111111111111111111111111111111111111111",
                    "address": "0x1111111111111111111111111111111111111111",
                    "type": "STABLE",
                    "protocolVersion": 3,
                    "factory": "0x2222222222222222222222222222222222222222",
                    "chain": "MAINNET",
                    "poolTokens": [
                        {
                            "address": "0x3333333333333333333333333333333333333333",
                            "decimals": 18,
                            "weight": null
                        }
                    ],
                    "dynamicData": {
                        "swapEnabled": true
                    },
                    "createTime": 1234567890,
                    "hook": {
                        "address": "0x4444444444444444444444444444444444444444",
                        "params": {
                            "maxSurgeFeePercentage": "0.95",
                            "surgeThresholdPercentage": "0.3"
                        }
                    }
                }
            ]
        }"#;

        let data: pools_query::Data = serde_json::from_str(json).unwrap();
        assert_eq!(data.aggregator_pools.len(), 1);
        let pool = &data.aggregator_pools[0];

        // Verify pool basic data
        assert_eq!(pool.pool_type_enum(), PoolType::Stable);
        assert!(pool.swap_enabled());

        // Verify hook data using direct field access (consistent with other pool types)
        assert!(pool.hook.is_some());
        let hook = pool.hook.as_ref().unwrap();
        assert_eq!(hook.address, H160([0x44; 20]));

        // Verify StableSurge parameters using direct field access
        assert!(hook.params.is_some());
        let params = hook.params.as_ref().unwrap();
        assert_eq!(
            params.max_surge_fee_percentage.unwrap(),
            "0.95".parse::<Bfp>().unwrap()
        );
        assert_eq!(
            params.surge_threshold_percentage.unwrap(),
            "0.3".parse::<Bfp>().unwrap()
        );
    }

    #[test]
    fn groups_pools_by_factory() {
        let pool1 = PoolData {
            id: "0x1111111111111111111111111111111111111111".to_string(),
            address: H160([0x11; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0x22; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![],
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
            max_trade_size_ratio: None,
            hook: None,
        };
        let pool2 = PoolData {
            id: "0x2222222222222222222222222222222222222222".to_string(),
            address: H160([0x22; 20]),
            pool_type: "WEIGHTED".to_string(),
            protocol_version: 3,
            factory: H160([0x22; 20]),
            chain: GqlChain::MAINNET,
            pool_tokens: vec![],
            dynamic_data: DynamicData {
                swap_enabled: false,
            },
            create_time: 1234567891,
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
            max_trade_size_ratio: None,
            hook: None,
        };
        let pools = RegisteredPools {
            fetched_block_number: 0,
            pools: vec![pool1.clone(), pool2.clone()],
        };
        let grouped = pools.group_by_factory();
        assert_eq!(grouped.len(), 1);
        let group = grouped.get(&H160([0x22; 20])).unwrap();
        assert_eq!(group.pools.len(), 2);
        assert!(group.pools.contains(&pool1));
        assert!(group.pools.contains(&pool2));
    }
}
