//! Module containing the Balancer API v3 client used for retrieving Balancer
//! pools.
//!
//! The pools retrieved from this client are used to prime the graph event store
//! to reduce start-up time. We do not use this in general for retrieving pools
//! as to:
//! - not rely on external services
//! - ensure that we are using the latest up-to-date pool data by using events
//!   from the node

use {
    super::swap::{fixed_point::Bfp, signed_fixed_point::SBfp},
    crate::subgraph::SubgraphClient,
    anyhow::{Context, Result},
    ethcontract::{H160, H256},
    reqwest::{Client, Url},
    serde::{Deserialize, Deserializer, Serialize},
    serde_json::json,
    serde_with::{DisplayFromStr, serde_as},
    std::collections::HashMap,
};

const QUERY_PAGE_SIZE: usize = 100;

/// Custom deserializer that converts empty strings to None for optional SBfp
/// fields. This fixes the inconsistency in the V2 API where 2-CLP pools return
/// empty strings instead of null for unused E-CLP parameters. Also handles
/// automatic precision detection for high-precision decimal values.
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
            use crate::sources::balancer_v2::swap::signed_fixed_point::FixedPointPrecision;

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

fn deserialize_optional_bfp<'de, D>(deserializer: D) -> Result<Option<Bfp>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(value) => {
            if value == "0" || value.is_empty() {
                Ok(None)
            } else {
                let parsed_value = value.parse::<f64>().map_err(serde::de::Error::custom)?;
                let scaled_value = (parsed_value * 1e18) as u64;
                Ok(Some(Bfp::from_wei(scaled_value.into())))
            }
        }
        None => Ok(None),
    }
}

/// Balancer API v3 client for fetching pool data.
pub struct BalancerApiClient {
    client: SubgraphClient,
    chain: GqlChain,
}

/// Supported chains in Balancer API v3.
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
    /// Creates a new Balancer API v3 client.
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

        // Use offset-based pagination with Balancer API v3
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
                            "chainIn": [self.chain],
                            "poolTypeIn": ["WEIGHTED", "STABLE", "LIQUIDITY_BOOTSTRAPPING", "COMPOSABLE_STABLE", "GYROE", "GYRO"],
                            "protocolVersionIn": [2]
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
            fetched_block_number: 0, // Balancer API v3 doesn't support historical queries
            pools,
        })
    }
}

/// Result of the registered pool query.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct RegisteredPools {
    /// The block number that the data was fetched. Set to 0 for Balancer API v3
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

/// Pool data from the Balancer API v3.
#[serde_as]
#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PoolData {
    pub id: String, // Can be 32-byte (V2) or 20-byte (V3) hex string
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
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub sqrt_alpha: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_sbfp")]
    pub sqrt_beta: Option<SBfp>,
    #[serde(default, deserialize_with = "deserialize_optional_bfp")]
    pub root3_alpha: Option<Bfp>,
}

/// Dynamic data for pools from Balancer API v3.
#[derive(Debug, Deserialize, Eq, PartialEq)]
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

/// Supported pool kinds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash)]
pub enum PoolType {
    Stable,
    Weighted,
    LiquidityBootstrapping,
    ComposableStable,
    Gyro2CLP,
    Gyro3CLP,
    GyroE,
}

impl PoolData {
    /// Converts the API pool type string to our internal enum.
    pub fn pool_type_enum(&self) -> PoolType {
        match self.pool_type.as_str() {
            "WEIGHTED" => PoolType::Weighted,
            "STABLE" => PoolType::Stable,
            "LIQUIDITY_BOOTSTRAPPING" => PoolType::LiquidityBootstrapping,
            "COMPOSABLE_STABLE" => PoolType::ComposableStable,
            "GYRO" => PoolType::Gyro2CLP,
            "GYRO3" => PoolType::Gyro3CLP,
            "GYROE" => PoolType::GyroE,
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

    /// Converts the string ID to H256. For V2 pools, this should be a 32-byte
    /// hex string. For V3 pools, this would be a 20-byte address, but we
    /// only support V2 pools.
    pub fn id_as_h256(&self) -> Result<H256> {
        // Remove 0x prefix if present
        let id_str = self.id.trim_start_matches("0x");

        // For V2 pools, we expect 32 bytes (64 hex characters)
        if id_str.len() == 64 {
            let id_bytes = hex::decode(id_str).context("Failed to decode pool ID as hex")?;
            Ok(H256::from_slice(&id_bytes))
        } else {
            Err(anyhow::anyhow!(
                "Invalid pool ID length for V2 pool: {} (expected 64 hex chars, got {})",
                self.id,
                id_str.len()
            ))
        }
    }

    /// Returns true if this is a V2 pool (protocol version 2).
    pub fn is_v2_pool(&self) -> bool {
        self.protocol_version == 2
    }
}

mod pools_query {
    use {super::PoolData, serde::Deserialize};

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
                sqrtAlpha
                sqrtBeta
                root3Alpha
            }
        }
    "#;

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    pub struct Data {
        #[serde(rename = "aggregatorPools")]
        pub aggregator_pools: Vec<PoolData>,
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::sources::balancer_v2::swap::fixed_point::Bfp, ethcontract::H160};

    #[test]
    fn decode_pools_data() {
        use pools_query::*;

        assert_eq!(
            serde_json::from_value::<Data>(json!({
                "aggregatorPools": [
                    {
                        "type": "WEIGHTED",
                        "address": "0x2222222222222222222222222222222222222222",
                        "id": "0x1111111111111111111111111111111111111111111111111111111111111111",
                        "protocolVersion": 2,
                        "factory": "0x5555555555555555555555555555555555555555",
                        "chain": "GNOSIS",
                        "poolTokens": [
                            {
                                "address": "0x3333333333333333333333333333333333333333",
                                "decimals": 3,
                                "weight": "0.5"
                            },
                            {
                                "address": "0x4444444444444444444444444444444444444444",
                                "decimals": 4,
                                "weight": "0.5"
                            },
                        ],
                        "dynamicData": {
                            "swapEnabled": true
                        },
                        "createTime": 1234567890
                    },
                    {
                        "type": "STABLE",
                        "address": "0x2222222222222222222222222222222222222222",
                        "id": "0x1111111111111111111111111111111111111111111111111111111111111111",
                        "protocolVersion": 2,
                        "factory": "0x5555555555555555555555555555555555555555",
                        "chain": "GNOSIS",
                        "poolTokens": [
                            {
                                "address": "0x3333333333333333333333333333333333333333",
                                "decimals": 3,
                            },
                            {
                                "address": "0x4444444444444444444444444444444444444444",
                                "decimals": 4,
                            },
                        ],
                        "dynamicData": {
                            "swapEnabled": true
                        },
                        "createTime": 1234567890
                    },
                    {
                        "type": "LIQUIDITY_BOOTSTRAPPING",
                        "address": "0x2222222222222222222222222222222222222222",
                        "id": "0x1111111111111111111111111111111111111111111111111111111111111111",
                        "protocolVersion": 2,
                        "factory": "0x5555555555555555555555555555555555555555",
                        "chain": "GNOSIS",
                        "poolTokens": [
                            {
                                "address": "0x3333333333333333333333333333333333333333",
                                "decimals": 3,
                                "weight": "0.5"
                            },
                            {
                                "address": "0x4444444444444444444444444444444444444444",
                                "decimals": 4,
                                "weight": "0.5"
                            },
                        ],
                        "dynamicData": {
                            "swapEnabled": true
                        },
                        "createTime": 1234567890
                    },
                    {
                        "type": "COMPOSABLE_STABLE",
                        "address": "0x2222222222222222222222222222222222222222",
                        "id": "0x1111111111111111111111111111111111111111111111111111111111111111",
                        "protocolVersion": 2,
                        "factory": "0x5555555555555555555555555555555555555555",
                        "chain": "GNOSIS",
                        "poolTokens": [
                            {
                                "address": "0x3333333333333333333333333333333333333333",
                                "decimals": 3,
                            },
                            {
                                "address": "0x4444444444444444444444444444444444444444",
                                "decimals": 4,
                            },
                        ],
                        "dynamicData": {
                            "swapEnabled": true
                        },
                        "createTime": 1234567890
                    },
                ],
            }))
            .unwrap(),
            Data {
                aggregator_pools: vec![
                    PoolData {
                        id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                            .to_string(),
                        address: H160([0x22; 20]),
                        pool_type: "WEIGHTED".to_string(),
                        protocol_version: 2,
                        factory: H160([0x55; 20]),
                        chain: GqlChain::GNOSIS,
                        pool_tokens: vec![
                            Token {
                                address: H160([0x33; 20]),
                                decimals: 3,
                                weight: Some(Bfp::from_wei(500_000_000_000_000_000u128.into())),
                                price_rate_provider: None,
                            },
                            Token {
                                address: H160([0x44; 20]),
                                decimals: 4,
                                weight: Some(Bfp::from_wei(500_000_000_000_000_000u128.into())),
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
                        root3_alpha: None,
                    },
                    PoolData {
                        id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                            .to_string(),
                        address: H160([0x22; 20]),
                        pool_type: "STABLE".to_string(),
                        protocol_version: 2,
                        factory: H160([0x55; 20]),
                        chain: GqlChain::GNOSIS,
                        pool_tokens: vec![
                            Token {
                                address: H160([0x33; 20]),
                                decimals: 3,
                                weight: None,
                                price_rate_provider: None,
                            },
                            Token {
                                address: H160([0x44; 20]),
                                decimals: 4,
                                weight: None,
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
                        root3_alpha: None,
                    },
                    PoolData {
                        id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                            .to_string(),
                        address: H160([0x22; 20]),
                        pool_type: "LIQUIDITY_BOOTSTRAPPING".to_string(),
                        protocol_version: 2,
                        factory: H160([0x55; 20]),
                        chain: GqlChain::GNOSIS,
                        pool_tokens: vec![
                            Token {
                                address: H160([0x33; 20]),
                                decimals: 3,
                                weight: Some(Bfp::from_wei(500_000_000_000_000_000u128.into())),
                                price_rate_provider: None,
                            },
                            Token {
                                address: H160([0x44; 20]),
                                decimals: 4,
                                weight: Some(Bfp::from_wei(500_000_000_000_000_000u128.into())),
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
                        root3_alpha: None,
                    },
                    PoolData {
                        id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                            .to_string(),
                        address: H160([0x22; 20]),
                        pool_type: "COMPOSABLE_STABLE".to_string(),
                        protocol_version: 2,
                        factory: H160([0x55; 20]),
                        chain: GqlChain::GNOSIS,
                        pool_tokens: vec![
                            Token {
                                address: H160([0x33; 20]),
                                decimals: 3,
                                weight: None,
                                price_rate_provider: None,
                            },
                            Token {
                                address: H160([0x44; 20]),
                                decimals: 4,
                                weight: None,
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
                        root3_alpha: None,
                    },
                ],
            }
        );
    }

    #[test]
    fn decode_gyro_eclp_high_precision_data() {
        use pools_query::*;

        // Test with actual high-precision values like from your API
        let gyro_eclp_json = json!({
            "aggregatorPools": [
                {
                    "type": "GYROE",
                    "address": "0x80fd5bc9d4fA6C22132f8bb2d9d30B01c3336FB3",
                    "id": "0x1111111111111111111111111111111111111111111111111111111111111111",
                    "protocolVersion": 2,
                    "factory": "0x5555555555555555555555555555555555555555",
                    "chain": "GNOSIS",
                    "poolTokens": [],
                    "dynamicData": { "swapEnabled": true },
                    "createTime": 1740124250,
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
                    "dSq": "0.9999999999999999988662409334210612"
                }
            ]
        });

        let data: Data = serde_json::from_value(gyro_eclp_json).unwrap();
        let pool = &data.aggregator_pools[0];

        // Verify standard precision parameters parsed correctly
        assert!(pool.alpha.is_some());
        assert!(pool.beta.is_some());
        assert!(pool.c.is_some());
        assert!(pool.s.is_some());
        assert!(pool.lambda.is_some());

        // Verify high-precision parameters parsed correctly without truncation
        assert!(pool.tau_alpha_x.is_some());
        assert!(pool.tau_alpha_y.is_some());
        assert!(pool.tau_beta_x.is_some());
        assert!(pool.tau_beta_y.is_some());
        assert!(pool.u.is_some());
        assert!(pool.v.is_some());
        assert!(pool.w.is_some());
        assert!(pool.z.is_some());
        assert!(pool.d_sq.is_some());

        // Verify that high-precision values are preserved
        // tauAlphaX: "-0.17378533390904767196396190604716688"
        // This should be scaled by 1e38 and stored as I256
        let tau_alpha_x = pool.tau_alpha_x.unwrap();

        // Convert back to verify precision is preserved (for future validation)
        let _tau_alpha_x_bigint = tau_alpha_x.to_big_int();

        // The value should be approximately -0.173785... * 1e38
        // We can check that it's in the right ballpark
        assert!(tau_alpha_x.is_negative());

        // Verify w parameter with many decimals
        let w = pool.w.unwrap();
        assert!(w.is_positive());

        println!("Successfully parsed high-precision GyroECLP data:");
        println!("  tauAlphaX: {}", tau_alpha_x);
        println!("  w: {}", w);
    }

    #[test]
    fn decode_gyro_2clp_with_empty_string_params() {
        use pools_query::*;

        // Test that empty strings in Gyro parameters are converted to None (the actual
        // issue we're fixing)
        let gyro_2clp_json = json!({
            "aggregatorPools": [
                {
                    "type": "GYRO",
                    "address": "0xdac42eeb17758daa38caf9a3540c808247527ae3",
                    "id": "0xdac42eeb17758daa38caf9a3540c808247527ae3000200000000000000000a2b",
                    "protocolVersion": 2,
                    "factory": "0x5555555555555555555555555555555555555555",
                    "chain": "POLYGON",
                    "poolTokens": [
                        {
                            "address": "0x3333333333333333333333333333333333333333",
                            "decimals": 6
                        }
                    ],
                    "dynamicData": { "swapEnabled": true },
                    "createTime": 1674129549,
                    // E-CLP parameters should be None for 2-CLP pools
                    "alpha": "",
                    "beta": "",
                    "c": "",
                    "s": "",
                    "lambda": "",
                    "tauAlphaX": "",
                    "tauAlphaY": "",
                    "tauBetaX": "",
                    "tauBetaY": "",
                    "u": "",
                    "v": "",
                    "w": "",
                    "z": "",
                    "dSq": "",
                    // 2-CLP parameters should have valid values
                    "sqrtAlpha": "0.997496867163000167",
                    "sqrtBeta": "1.002496882788171068"
                }
            ]
        });

        let data: Data = serde_json::from_value(gyro_2clp_json).unwrap();
        let pool = &data.aggregator_pools[0];

        // Verify empty strings converted to None
        assert!(pool.alpha.is_none());
        assert!(pool.beta.is_none());
        assert!(pool.c.is_none());
        assert!(pool.s.is_none());
        assert!(pool.lambda.is_none());
        assert!(pool.tau_alpha_x.is_none());
        assert!(pool.tau_alpha_y.is_none());
        assert!(pool.tau_beta_x.is_none());
        assert!(pool.tau_beta_y.is_none());
        assert!(pool.u.is_none());
        assert!(pool.v.is_none());
        assert!(pool.w.is_none());
        assert!(pool.z.is_none());
        assert!(pool.d_sq.is_none());

        // Verify valid decimal strings parsed correctly
        assert!(pool.sqrt_alpha.is_some());
        assert!(pool.sqrt_beta.is_some());

        // Verify pool type identification
        assert_eq!(pool.pool_type_enum(), PoolType::Gyro2CLP);

        println!("Successfully parsed 2-CLP pool with empty E-CLP params:");
        println!("  sqrtAlpha: {:?}", pool.sqrt_alpha);
        println!("  sqrtBeta: {:?}", pool.sqrt_beta);
    }

    #[test]
    fn groups_pools_by_factory() {
        let pools = RegisteredPools {
            fetched_block_number: 42,
            pools: vec![
                PoolData {
                    id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                        .to_string(),
                    address: H160([0x22; 20]),
                    pool_type: "WEIGHTED".to_string(),
                    protocol_version: 2,
                    factory: H160([0x55; 20]),
                    chain: GqlChain::GNOSIS,
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
                    root3_alpha: None,
                },
                PoolData {
                    id: "0x2222222222222222222222222222222222222222222222222222222222222222"
                        .to_string(),
                    address: H160([0x33; 20]),
                    pool_type: "STABLE".to_string(),
                    protocol_version: 2,
                    factory: H160([0x55; 20]),
                    chain: GqlChain::GNOSIS,
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
                    root3_alpha: None,
                },
                PoolData {
                    id: "0x3333333333333333333333333333333333333333333333333333333333333333"
                        .to_string(),
                    address: H160([0x44; 20]),
                    pool_type: "WEIGHTED".to_string(),
                    protocol_version: 2,
                    factory: H160([0x66; 20]),
                    chain: GqlChain::GNOSIS,
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
                    root3_alpha: None,
                },
            ],
        };

        let grouped = pools.group_by_factory();
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[&H160([0x55; 20])].pools.len(), 2);
        assert_eq!(grouped[&H160([0x66; 20])].pools.len(), 1);
    }
}
