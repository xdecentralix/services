use {super::Response, tracing::Instrument};

mod dto;

use {crate::domain::solver::Solver, std::sync::Arc};

pub async fn solve(
    state: axum::extract::State<Arc<Solver>>,
    axum::extract::Json(auction): axum::extract::Json<dto::Auction>,
) -> (
    axum::http::StatusCode,
    axum::response::Json<Response<dto::Solutions>>,
) {
    let handle_request = async {
        let liquidity_client = state.liquidity_client();
        
        // Get base tokens and protocols from solver configuration if available
        let base_tokens = {
            let tokens: Vec<_> = state.base_tokens().iter().map(|t| t.0).collect();
            if tokens.is_empty() { None } else { Some(tokens) }
        };
        let protocols = state.protocols();
        
        let auction = match dto::auction::into_domain(
            auction, 
            liquidity_client, 
            base_tokens.as_deref(),
            protocols.as_deref()
        ).await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(?err, "invalid auction");
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(Response::Err(err)),
                );
            }
        };

        let auction_id = auction.id;
        let solutions = state
            .solve(auction)
            .instrument(tracing::info_span!("auction", id = %auction_id))
            .await;

        tracing::trace!(?auction_id, ?solutions);

        let solutions = dto::solution::from_domain(&solutions);
        (
            axum::http::StatusCode::OK,
            axum::response::Json(Response::Ok(solutions)),
        )
    };

    handle_request
        .instrument(tracing::info_span!("/solve"))
        .await
}
