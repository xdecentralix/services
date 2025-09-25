use {serde::Serialize, solvers_dto};

/// Response containing liquidity data for the requested token pairs
#[derive(Debug, Serialize)]
pub struct LiquidityResponse {
    /// The auction ID this liquidity data corresponds to
    pub auction_id: u64,

    /// All available liquidity sources for the requested pairs
    /// This includes data from all requested protocols
    pub liquidity: Vec<solvers_dto::auction::Liquidity>,

    /// Block number this data was fetched at
    pub block_number: u64,

    /// Timestamp when this data was generated (Unix timestamp)
    pub timestamp: u64,
}

/// Response wrapper used by the API infrastructure
#[derive(Debug, Serialize)]
pub struct ApiLiquidityResponse {
    pub result: LiquidityResponse,
}
