use {
    crate::domain::eth,
    reqwest::Client,
    serde::{Deserialize, Serialize},
    std::time::Duration,
    tracing,
};

/// HTTP client for fetching liquidity data from the liquidity-driver API
#[derive(Clone)]
pub struct LiquidityClient {
    client: Client,
    base_url: String,
    timeout: Duration,
}

impl LiquidityClient {
    pub fn new(base_url: String, timeout: Duration) -> Self {
        Self {
            client: Client::new(),
            base_url,
            timeout,
        }
    }

    /// Fetch liquidity data for the specified token pairs and protocols
    pub async fn fetch_liquidity(
        &self,
        request: LiquidityRequest,
    ) -> Result<LiquidityResponse, LiquidityClientError> {
        tracing::debug!(
            auction_id = request.auction_id,
            pairs_count = request.token_pairs.len(),
            "Fetching liquidity from driver API"
        );

        let response = self
            .client
            .post(&format!("{}/api/v1/liquidity", self.base_url))
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(LiquidityClientError::Http)?;

        if !response.status().is_success() {
            return Err(LiquidityClientError::HttpStatus(response.status()));
        }

        let api_response: ApiLiquidityResponse =
            response.json().await.map_err(LiquidityClientError::Json)?;

        tracing::debug!(
            auction_id = request.auction_id,
            liquidity_count = api_response.result.liquidity.len(),
            "Successfully fetched liquidity from driver API"
        );

        Ok(api_response.result)
    }
}

/// Request payload for the liquidity-driver API
#[derive(Debug, Serialize)]
pub struct LiquidityRequest {
    pub auction_id: u64,
    pub tokens: Vec<eth::H160>,
    pub token_pairs: Vec<(eth::H160, eth::H160)>,
    pub block_number: u64,
    pub protocols: Vec<String>,
}

/// Response from the liquidity-driver API
#[derive(Debug, Serialize, Deserialize)]
pub struct LiquidityResponse {
    pub auction_id: u64,
    pub liquidity: Vec<solvers_dto::auction::Liquidity>,
    pub block_number: u64,
    pub timestamp: u64,
}

/// Wrapper response from the API
#[derive(Debug, Deserialize)]
struct ApiLiquidityResponse {
    result: LiquidityResponse,
}

#[derive(Debug)]
pub enum LiquidityClientError {
    Http(reqwest::Error),
    HttpStatus(reqwest::StatusCode),
    Json(reqwest::Error),
}

impl std::fmt::Display for LiquidityClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiquidityClientError::Http(e) => write!(f, "HTTP request failed: {}", e),
            LiquidityClientError::HttpStatus(status) => {
                write!(f, "HTTP request returned status: {}", status)
            }
            LiquidityClientError::Json(e) => write!(f, "Failed to parse JSON response: {}", e),
        }
    }
}

impl std::error::Error for LiquidityClientError {}

impl From<reqwest::Error> for LiquidityClientError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            LiquidityClientError::Json(e)
        } else {
            LiquidityClientError::Http(e)
        }
    }
}
