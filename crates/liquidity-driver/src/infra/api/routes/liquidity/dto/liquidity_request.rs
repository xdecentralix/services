use {crate::domain::eth, serde::Deserialize};

/// Request for fetching liquidity data for specific token pairs
#[derive(Debug, Deserialize)]
pub struct LiquidityRequest {
    /// Unique identifier for the auction this liquidity is being fetched for
    pub auction_id: u64,
    
    /// All tokens involved in the auction
    pub tokens: Vec<eth::H160>,
    
    /// Specific trading pairs that need liquidity data
    /// These pairs will be automatically expanded with base token routing
    pub token_pairs: Vec<(eth::H160, eth::H160)>,
    
    /// Block number for ensuring data freshness and consistency  
    pub block_number: u64,
    
    /// List of protocols to fetch liquidity from
    /// e.g., ["balancer_v2", "uniswap_v2", "uniswap_v3"]
    pub protocols: Vec<String>,
}


