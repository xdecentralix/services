use {super::Response, tracing::Instrument};

mod dto;

use {crate::domain::solver::Solver, std::sync::Arc};

pub async fn solve(
    state: axum::extract::State<Arc<Solver>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(auction): axum::extract::Json<dto::Auction>,
) -> (
    axum::http::StatusCode,
    axum::response::Json<Response<dto::Solutions>>,
) {
    let handle_request = async {
        // 🔍 LOG RAW REQUEST DATA FROM COW PROTOCOL
        tracing::info!(
            auction_id = ?auction.id,
            orders_count = auction.orders.len(),
            "🎯 RECEIVED SOLVE REQUEST FROM COW PROTOCOL"
        );

        // Log request headers to identify source
        let user_agent = headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        let x_request_id = headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none");

        tracing::info!(
            user_agent = %user_agent,
            content_type = %content_type,
            request_id = %x_request_id,
            "📡 REQUEST HEADERS"
        );

        // Log detailed order information
        for (i, order) in auction.orders.iter().enumerate() {
            tracing::info!(
                order_index = i,
                sell_token = ?order.sell_token,
                buy_token = ?order.buy_token,
                sell_amount = ?order.sell_amount,
                buy_amount = ?order.buy_amount,
                kind = ?order.kind,
                "📝 ORDER DETAILS"
            );
        }

        // Log raw auction structure (be careful with size)
        if auction.orders.len() <= 5 {
            tracing::debug!(
                auction_json = ?serde_json::to_string(&auction).unwrap_or_else(|_| "serialization_failed".to_string()),
                "🔍 RAW AUCTION JSON (limited to ≤5 orders)"
            );
        } else {
            tracing::info!(
                orders_count = auction.orders.len(),
                "🔍 Large auction - not logging full JSON to avoid spam"
            );
        }
        let liquidity_client = state.liquidity_client();

        // Get base tokens and protocols from solver configuration if available
        let base_tokens = {
            let tokens: Vec<_> = state.base_tokens().iter().map(|t| t.0).collect();
            if tokens.is_empty() {
                None
            } else {
                Some(tokens)
            }
        };
        let protocols = state.protocols();

        // Serialize auction DTO for potential saving later (before consuming it)
        let auction_json = serde_json::to_value(&auction).ok();

        let auction = match dto::auction::into_domain(
            auction,
            liquidity_client,
            base_tokens.as_deref(),
            protocols.as_deref(),
        )
        .await
        {
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

        tracing::info!(
            auction_id = %auction_id,
            solutions_count = solutions.len(),
            "🔄 COMPUTED SOLUTIONS FOR COW PROTOCOL"
        );

        // Log each solution summary
        for (i, solution) in solutions.iter().enumerate() {
            tracing::info!(
                solution_index = i,
                solution_id = ?solution.id,
                trades_count = solution.trades.len(),
                interactions_count = solution.interactions.len(),
                "💡 SOLUTION SUMMARY"
            );
        }

        let solutions_dto = dto::solution::from_domain(&solutions);

        tracing::info!(
            auction_id = %auction_id,
            returning_solutions = solutions_dto.solutions.len(),
            "✅ SENDING RESPONSE TO COW PROTOCOL"
        );

        // Save auction and solutions to JSON if configured (non-blocking)
        if let (Some(save_dir), Some(auction_json)) = (state.auction_save_directory(), auction_json)
        {
            let solutions_json = serde_json::to_value(&solutions_dto).ok();
            let save_dir = save_dir.to_path_buf();
            tokio::spawn(async move {
                if let Some(solutions) = solutions_json {
                    save_auction_and_solutions(auction_json, solutions, &save_dir).await;
                }
            });
        }

        (
            axum::http::StatusCode::OK,
            axum::response::Json(Response::Ok(solutions_dto)),
        )
    };

    handle_request
        .instrument(tracing::info_span!("/solve"))
        .await
}

/// Saves auction and solutions to a JSON file in the configured directory.
/// This function runs in a background task and logs errors without failing the
/// request.
async fn save_auction_and_solutions(
    auction: serde_json::Value,
    solutions: serde_json::Value,
    save_dir: &std::path::Path,
) {
    use tokio::fs;

    // Determine filename based on auction ID
    let filename = match auction.get("id").and_then(|v| v.as_str()) {
        Some(id) => format!("{}.json", id),
        None => {
            // Use timestamp for quote auctions
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            format!("quote_{}.json", timestamp)
        }
    };

    let file_path = save_dir.join(&filename);

    // Create directory if it doesn't exist
    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(
            ?err,
            directory = ?save_dir,
            "Failed to create auction save directory"
        );
        return;
    }

    // Create combined JSON structure
    let combined = serde_json::json!({
        "auction": auction,
        "solutions": solutions,
    });

    // Serialize to pretty JSON
    let json_content = match serde_json::to_string_pretty(&combined) {
        Ok(content) => content,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize auction/solutions to JSON");
            return;
        }
    };

    // Write to file
    let solutions_count = solutions
        .get("solutions")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    match fs::write(&file_path, json_content).await {
        Ok(_) => {
            tracing::info!(
                file_path = ?file_path,
                auction_id = ?auction.get("id"),
                solutions_count,
                "💾 Saved auction and solutions to JSON file"
            );
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                file_path = ?file_path,
                "Failed to write auction/solutions JSON file"
            );
        }
    }
}
