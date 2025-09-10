use {
    crate::{
        domain::liquidity,
        infra::{
            api::{error, State},
            liquidity::fetcher::AtBlock,
            observe,
        },
    },
    std::{collections::HashSet, str::FromStr},
    tracing::Instrument,
};

mod dto;

pub use dto::*;

/// Register the liquidity route with the router
pub(in crate::infra::api) fn liquidity(router: axum::Router<State>) -> axum::Router<State> {
    router.route("/api/v1/liquidity", axum::routing::post(route))
}

/// Main handler for the /api/v1/liquidity endpoint
async fn route(
    state: axum::extract::State<State>,
    req: axum::Json<LiquidityRequest>,
) -> Result<axum::Json<ApiLiquidityResponse>, (hyper::StatusCode, axum::Json<error::Error>)> {
    let auction_id = req.auction_id; // Extract before moving req
    
    let handle_request = async {
        let request = req.0;
        
        // Convert token pairs to the domain format
        let pairs = request
            .token_pairs
            .into_iter()
            .map(|(a, b)| liquidity::TokenPair::try_new(a.into(), b.into()))
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|_| LiquidityError::InvalidTokenPair)?;

        observe::fetching_liquidity();
        
        // Fetch liquidity using the existing liquidity fetcher
        let domain_liquidity = state
            .liquidity()
            .fetch(&pairs, AtBlock::Latest)
            .await;

        observe::fetched_liquidity(&domain_liquidity);
        
        // Convert domain liquidity to solvers-dto format
        let liquidity_dto = domain_liquidity
            .into_iter()
            .filter_map(|liq| match convert_domain_to_dto(liq) {
                Ok(dto) => Some(dto),
                Err(e) => {
                    tracing::warn!(
                        liquidity_id = ?e,
                        "Failed to convert domain liquidity to DTO, skipping"
                    );
                    None
                }
            })
            .collect();

        let response = LiquidityResponse {
            auction_id: request.auction_id,
            liquidity: liquidity_dto,
            block_number: request.block_number,
            timestamp: chrono::Utc::now().timestamp() as u64,
        };

        Ok(axum::Json(ApiLiquidityResponse {
            result: response,
        }))
    };

    handle_request
        .instrument(tracing::info_span!(
            "/api/v1/liquidity",
            auction_id = auction_id
        ))
        .await
}

/// Convert domain liquidity types to solvers_dto types
fn convert_domain_to_dto(
    liquidity: liquidity::Liquidity,
) -> Result<solvers_dto::auction::Liquidity, LiquidityError> {
    use std::collections::HashMap;
    
    match liquidity.kind {
        liquidity::Kind::UniswapV2(pool) => {
            let mut tokens = HashMap::new();
            let reserves_iter = pool.reserves.iter();
            let assets: Vec<_> = reserves_iter.collect();
            
            if assets.len() != 2 {
                return Err(LiquidityError::UnsupportedPoolType);
            }
            
            tokens.insert(
                assets[0].token.0.into(),
                solvers_dto::auction::ConstantProductReserve {
                    balance: assets[0].amount.into(),
                }
            );
            tokens.insert(
                assets[1].token.0.into(),
                solvers_dto::auction::ConstantProductReserve {
                    balance: assets[1].amount.into(),
                }
            );
            
            Ok(solvers_dto::auction::Liquidity::ConstantProduct(
                solvers_dto::auction::ConstantProductPool {
                    id: liquidity.id.0.to_string(),
                    address: pool.address.0.into(),
                    router: pool.router.0.into(),
                    gas_estimate: liquidity.gas.0.into(),
                    tokens,
                    fee: bigdecimal::BigDecimal::from_str("0.003")
                        .unwrap_or_else(|_| bigdecimal::BigDecimal::from(3) / bigdecimal::BigDecimal::from(1000)),
                }
            ))
        }
        
        // Add more conversions for other pool types as needed
        // For now, return unsupported for other types
        _ => {
            tracing::debug!(
                liquidity_type = ?std::mem::discriminant(&liquidity.kind),
                "Unsupported liquidity type for DTO conversion, skipping"
            );
            Err(LiquidityError::UnsupportedPoolType)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LiquidityError {
    #[error("Invalid token pair")]
    InvalidTokenPair,
    #[error("Unsupported pool type")]
    UnsupportedPoolType,
}

impl From<LiquidityError> for (hyper::StatusCode, axum::Json<error::Error>) {
    fn from(error: LiquidityError) -> Self {
        tracing::warn!(?error, "Liquidity API error");
        
        // Map to existing error kinds that are exposed via the error module
        match error {
            LiquidityError::InvalidTokenPair => {
                // Use the existing From implementation for InvalidTokens error kind
                let auction_error = crate::infra::api::routes::AuctionError::InvalidTokens;
                auction_error.into()
            },
            LiquidityError::UnsupportedPoolType => {
                // For now, just return the same error as InvalidTokens since they both
                // result in Bad Request. We can make this more specific later if needed.
                let auction_error = crate::infra::api::routes::AuctionError::InvalidTokens;
                auction_error.into()
            },
        }
    }
}
