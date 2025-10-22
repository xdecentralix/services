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

        let (auction, fetched_liquidity) = match dto::auction::into_domain(
            auction,
            liquidity_client,
            base_tokens.as_deref(),
            protocols.as_deref(),
            state.auction_save_directory(),
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
            let save_dir_for_competition = save_dir.clone();
            let save_dir_for_enhanced = save_dir.clone();
            
            tokio::spawn(async move {
                if let Some(solutions) = solutions_json {
                    save_auction_and_solutions(auction_json, solutions, &save_dir).await;
                }
            });

            // Spawn background task to fetch competition data
            let cow_api_url = state.cow_api_base_url();
            tokio::spawn(async move {
                fetch_and_save_competition_data(auction_id, cow_api_url, &save_dir_for_competition).await;
            });

            // Spawn background task to create enhanced solutions if liquidity was fetched
            if let Some(liq_response) = fetched_liquidity {
                // Serialize solutions_dto for the enhanced file before returning
                let solutions_json_for_enhanced = serde_json::to_value(&solutions_dto).ok();
                tokio::spawn(async move {
                    if let Some(solutions_json) = solutions_json_for_enhanced {
                        // Deserialize back to Solutions for the function
                        if let Ok(solutions_for_enhance) = serde_json::from_value(solutions_json) {
                            save_enhanced_solutions(
                                solutions_for_enhance,
                                liq_response,
                                auction_id,
                                &save_dir_for_enhanced,
                            ).await;
                        }
                    }
                });
            }
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

/// Saves auction and solutions to separate JSON files in the configured directory.
/// This function runs in a background task and logs errors without failing the
/// request.
async fn save_auction_and_solutions(
    auction: serde_json::Value,
    solutions: serde_json::Value,
    save_dir: &std::path::Path,
) {
    use tokio::fs;

    // Determine base filename based on auction ID
    let base_filename = match auction.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            // Use timestamp for quote auctions
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            format!("quote_{}", timestamp)
        }
    };

    let auction_file_path = save_dir.join(format!("{}_auction.json", base_filename));
    let solutions_file_path = save_dir.join(format!("{}_solutions.json", base_filename));

    // Create directory if it doesn't exist
    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(
            ?err,
            directory = ?save_dir,
            "Failed to create auction save directory"
        );
        return;
    }

    // Serialize auction to pretty JSON
    let auction_json = match serde_json::to_string_pretty(&auction) {
        Ok(content) => content,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize auction to JSON");
            return;
        }
    };

    // Serialize solutions to pretty JSON
    let solutions_json = match serde_json::to_string_pretty(&solutions) {
        Ok(content) => content,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize solutions to JSON");
            return;
        }
    };

    let solutions_count = solutions
        .get("solutions")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Write auction file
    let auction_write_result = fs::write(&auction_file_path, auction_json).await;
    
    // Write solutions file
    let solutions_write_result = fs::write(&solutions_file_path, solutions_json).await;

    // Log results
    match (auction_write_result, solutions_write_result) {
        (Ok(_), Ok(_)) => {
            tracing::info!(
                auction_file = ?auction_file_path,
                solutions_file = ?solutions_file_path,
                auction_id = ?auction.get("id"),
                solutions_count,
                "💾 Saved auction and solutions to separate JSON files"
            );
        }
        (Err(err), _) => {
            tracing::warn!(
                ?err,
                file_path = ?auction_file_path,
                "Failed to write auction JSON file"
            );
        }
        (_, Err(err)) => {
            tracing::warn!(
                ?err,
                file_path = ?solutions_file_path,
                "Failed to write solutions JSON file"
            );
        }
    }
}

/// Fetches competition data from the CoW API and saves it to a JSON file.
/// This function waits 60 seconds before attempting to fetch, then retries up to 10 times.
async fn fetch_and_save_competition_data(
    auction_id: crate::domain::auction::Id,
    cow_api_base_url: &str,
    save_dir: &std::path::Path,
) {
    use tokio::fs;
    use tokio::time::{sleep, Duration};

    // Extract the numeric auction ID
    let auction_id_num = match auction_id {
        crate::domain::auction::Id::Solve(id) => id,
        crate::domain::auction::Id::Quote => {
            tracing::debug!("Skipping competition data fetch for quote auction");
            return;
        }
    };

    // Wait 60 seconds for the competition to settle
    tracing::info!(
        auction_id = auction_id_num,
        "Waiting 60 seconds before fetching competition data"
    );
    sleep(Duration::from_secs(60)).await;

    let url = format!(
        "{}/api/v2/solver_competition/{}",
        cow_api_base_url, auction_id_num
    );
    let client = reqwest::Client::new();

    // Retry up to 10 times with 10 second delays between attempts
    for attempt in 1..=10 {
        tracing::debug!(
            auction_id = auction_id_num,
            attempt,
            "Fetching competition data"
        );

        match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<serde_json::Value>().await {
                        Ok(competition_data) => {
                            // Save to file
                            let filename = format!("{}_competition.json", auction_id_num);
                            let file_path = save_dir.join(filename);

                            // Create directory if needed
                            if let Err(err) = fs::create_dir_all(save_dir).await {
                                tracing::warn!(
                                    ?err,
                                    directory = ?save_dir,
                                    "Failed to create competition data directory"
                                );
                                return;
                            }

                            // Serialize to pretty JSON
                            let json_string = match serde_json::to_string_pretty(&competition_data)
                            {
                                Ok(s) => s,
                                Err(err) => {
                                    tracing::warn!(
                                        ?err,
                                        "Failed to serialize competition data"
                                    );
                                    return;
                                }
                            };

                            // Write to file
                            match fs::write(&file_path, json_string).await {
                                Ok(_) => {
                                    tracing::info!(
                                        auction_id = auction_id_num,
                                        file_path = ?file_path,
                                        attempt,
                                        "💾 Successfully saved competition data"
                                    );
                                    return; // Success!
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        ?err,
                                        file_path = ?file_path,
                                        "Failed to write competition data file"
                                    );
                                    return;
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                ?err,
                                auction_id = auction_id_num,
                                attempt,
                                "Failed to parse competition data JSON"
                            );
                        }
                    }
                } else if response.status().as_u16() == 404 {
                    tracing::debug!(
                        auction_id = auction_id_num,
                        attempt,
                        "Competition data not yet available (404), will retry"
                    );
                } else {
                    tracing::warn!(
                        auction_id = auction_id_num,
                        status = response.status().as_u16(),
                        attempt,
                        "Unexpected HTTP status when fetching competition data"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    ?err,
                    auction_id = auction_id_num,
                    attempt,
                    "HTTP request failed when fetching competition data"
                );
            }
        }

        // Wait 10 seconds before next retry (unless this was the last attempt)
        if attempt < 10 {
            sleep(Duration::from_secs(10)).await;
        }
    }

    tracing::warn!(
        auction_id = auction_id_num,
        "Failed to fetch competition data after 10 attempts"
    );
}

/// Saves enhanced solutions with embedded liquidity details
async fn save_enhanced_solutions(
    solutions: dto::Solutions,
    liquidity_response: crate::infra::liquidity_client::LiquidityResponse,
    auction_id: crate::domain::auction::Id,
    save_dir: &std::path::Path,
) {
    use tokio::fs;
    
    let auction_id_num = match auction_id {
        crate::domain::auction::Id::Solve(id) => id,
        crate::domain::auction::Id::Quote => {
            tracing::debug!("Skipping enhanced solutions for quote auction");
            return;
        }
    };
    
    // Create enhanced solutions
    let enhanced = dto::auction::create_enhanced_solutions(&solutions, &liquidity_response);
    
    let filename = format!("{}_enhanced_solutions.json", auction_id_num);
    let file_path = save_dir.join(filename);
    
    // Create directory if needed
    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(?err, directory = ?save_dir, "Failed to create directory");
        return;
    }
    
    // Serialize to pretty JSON
    let json_string = match serde_json::to_string_pretty(&enhanced) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize enhanced solutions");
            return;
        }
    };
    
    // Write to file
    match fs::write(&file_path, json_string).await {
        Ok(_) => {
            tracing::info!(
                auction_id = auction_id_num,
                file_path = ?file_path,
                "💾 Saved enhanced solutions with liquidity details"
            );
        }
        Err(err) => {
            tracing::warn!(?err, file_path = ?file_path, "Failed to write enhanced solutions");
        }
    }
}
