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
    super::swap::fixed_point::Bfp,
    crate::subgraph::SubgraphClient,
    anyhow::{Context, Result},
    ethcontract::{H160, H256},
    reqwest::{Client, Url},
    serde::{Deserialize, Serialize},
    serde_json::json,
    serde_with::{DisplayFromStr, serde_as},
    std::collections::HashMap,
};

const QUERY_PAGE_SIZE: usize = 100;

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
                            "poolTypeIn": ["WEIGHTED", "STABLE", "LIQUIDITY_BOOTSTRAPPING", "COMPOSABLE_STABLE"],
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
}

impl PoolData {
    /// Converts the API pool type string to our internal enum.
    pub fn pool_type_enum(&self) -> PoolType {
        match self.pool_type.as_str() {
            "WEIGHTED" => PoolType::Weighted,
            "STABLE" => PoolType::Stable,
            "LIQUIDITY_BOOTSTRAPPING" => PoolType::LiquidityBootstrapping,
            "COMPOSABLE_STABLE" => PoolType::ComposableStable,
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
                    },
                ],
            }
        );
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
                },
            ],
        };

        let grouped = pools.group_by_factory();
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[&H160([0x55; 20])].pools.len(), 2);
        assert_eq!(grouped[&H160([0x66; 20])].pools.len(), 1);
    }
}
