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

use {
    super::swap::fixed_point::Bfp,
    crate::subgraph::SubgraphClient,
    anyhow::{Context, Result},
    ethcontract::H160,
    reqwest::{Client, Url},
    serde::{Deserialize, Serialize},
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
                            "chainIn": [self.chain],
                            "poolTypeIn": ["WEIGHTED", "STABLE"],
                            "protocolVersionIn": [3] // V3 protocol
                        }),
                    }),
                )
                .await?
                .pool_get_pools;

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
    Weighted, // BalancerV3WeightedPoolFactory
    Stable,   // BalancerV3StablePoolFactory, BalancerV3StablePoolFactoryV2
}

impl PoolData {
    /// Converts the API pool type string to our internal enum.
    pub fn pool_type_enum(&self) -> PoolType {
        match self.pool_type.as_str() {
            "WEIGHTED" => PoolType::Weighted,
            "STABLE" => PoolType::Stable,
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
        query PoolGetPools(
            $first: Int,
            $skip: Int,
            $orderBy: GqlPoolOrderBy,
            $orderDirection: GqlPoolOrderDirection,
            $where: GqlPoolFilter
        ) {
            poolGetPools(
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

    #[derive(Debug, Deserialize)]
    pub struct Data {
        #[serde(rename = "poolGetPools")]
        pub pool_get_pools: Vec<super::PoolData>,
    }
}

#[cfg(test)]
mod tests {
    use {super::*, ethcontract::H160};

    #[test]
    fn decode_pools_data() {
        let json = r#"{
            "poolGetPools": [
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
                    "createTime": 1234567890
                }
            ]
        }"#;

        let data: pools_query::Data = serde_json::from_str(json).unwrap();
        assert_eq!(data.pool_get_pools.len(), 1);
        let pool = &data.pool_get_pools[0];
        assert_eq!(pool.id, "0x1111111111111111111111111111111111111111");
        assert_eq!(pool.address, H160([0x11; 20]));
        assert_eq!(pool.pool_type_enum(), PoolType::Weighted);
        assert!(pool.swap_enabled());
        assert_eq!(pool.tokens().len(), 1);
        assert_eq!(pool.tokens()[0].address, H160([0x33; 20]));
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
