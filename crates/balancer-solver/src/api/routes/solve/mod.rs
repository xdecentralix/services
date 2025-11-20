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

        // Create swap logger if auction save directory is configured
        let swap_logger = state
            .auction_save_directory()
            .map(|_| crate::boundary::swap_logger::SwapLogger::new());

        let solutions = state
            .solve_with_logger(auction, swap_logger.clone())
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
            let save_dir_for_verify = save_dir.clone();
            let save_dir_for_swap_log = save_dir.clone();
            let save_dir_for_swap_log_verify = save_dir.clone();

            tokio::spawn(async move {
                if let Some(solutions) = solutions_json {
                    save_auction_and_solutions(auction_json, solutions, &save_dir).await;
                }
            });

            // Save swap log if logger was used, and optionally verify it
            if let Some(logger) = swap_logger {
                let swap_records = logger.get_records();
                if !swap_records.is_empty() {
                    let auction_id_num = match auction_id {
                        crate::domain::auction::Id::Solve(id) => Some(id),
                        crate::domain::auction::Id::Quote => None,
                    };
                    let verifier_for_swap_log = state.verifier().cloned();

                    tokio::spawn(async move {
                        save_swap_log(swap_records.clone(), auction_id_num, &save_dir_for_swap_log)
                            .await;

                        // Verify swap log if verifier is configured
                        if let Some(verifier) = verifier_for_swap_log {
                            verify_and_save_swap_log(
                                swap_records,
                                auction_id_num,
                                verifier,
                                &save_dir_for_swap_log_verify,
                            )
                            .await;
                        }
                    });
                }
            }

            // Spawn background task to fetch competition data
            let cow_api_url = state.cow_api_base_url();
            tokio::spawn(async move {
                fetch_and_save_competition_data(auction_id, cow_api_url, &save_dir_for_competition)
                    .await;
            });

            // Spawn background task to create enhanced solutions if liquidity was fetched
            // If verifier is also configured, verify using the enhanced solutions
            if let Some(liq_response) = fetched_liquidity {
                let verifier_opt = state.verifier().cloned();
                let solutions_json_for_enhanced = serde_json::to_value(&solutions_dto).ok();

                tokio::spawn(async move {
                    if let Some(solutions_json) = solutions_json_for_enhanced {
                        // Deserialize back to Solutions for the function
                        if let Ok(solutions_for_enhance) =
                            serde_json::from_value::<dto::Solutions>(solutions_json)
                        {
                            // Create enhanced solutions with liquidityDetails
                            let enhanced = dto::auction::create_enhanced_solutions(
                                &solutions_for_enhance,
                                &liq_response,
                            );

                            // Save enhanced solutions file
                            save_enhanced_solutions_json(
                                enhanced.clone(),
                                auction_id,
                                &save_dir_for_enhanced,
                            )
                            .await;

                            // Verify using enhanced solutions if verifier is configured
                            if let Some(verifier) = verifier_opt {
                                verify_and_save_solutions(
                                    enhanced,
                                    verifier,
                                    auction_id,
                                    &save_dir_for_verify,
                                )
                                .await;
                            }
                        }
                    }
                });
            } else if let Some(verifier) = state.verifier() {
                // No liquidity fetched, but verifier configured - use basic solutions
                let solutions_json_for_verify = serde_json::to_value(&solutions_dto).ok();
                let verifier = verifier.clone();

                tokio::spawn(async move {
                    if let Some(solutions_json) = solutions_json_for_verify {
                        verify_and_save_solutions(
                            solutions_json,
                            verifier,
                            auction_id,
                            &save_dir_for_verify,
                        )
                        .await;
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

/// Saves swap log data to JSON file in the configured directory.
/// This function runs in a background task and logs errors without failing the
/// request.
async fn save_swap_log(
    swap_records: Vec<crate::boundary::swap_logger::SwapRecord>,
    auction_id: Option<i64>,
    save_dir: &std::path::Path,
) {
    use tokio::fs;

    // Determine filename based on auction ID
    let base_filename = match auction_id {
        Some(id) => id.to_string(),
        None => {
            // Use timestamp for quote auctions
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            format!("quote_{}", timestamp)
        }
    };

    let swap_log_file_path = save_dir.join(format!("{}_swap_log.json", base_filename));

    // Create directory if it doesn't exist
    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(
            ?err,
            directory = ?save_dir,
            "Failed to create swap log save directory"
        );
        return;
    }

    // Serialize swap log to pretty JSON
    let swap_log_json = match serde_json::to_string_pretty(&serde_json::json!({
        "auction_id": auction_id,
        "swaps_count": swap_records.len(),
        "swaps": swap_records,
    })) {
        Ok(content) => content,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize swap log to JSON");
            return;
        }
    };

    // Write swap log file
    match fs::write(&swap_log_file_path, swap_log_json).await {
        Ok(_) => {
            tracing::info!(
                swap_log_file = ?swap_log_file_path,
                auction_id = ?auction_id,
                swaps_count = swap_records.len(),
                "💾 Saved swap log to JSON file"
            );
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                file_path = ?swap_log_file_path,
                "Failed to write swap log JSON file"
            );
        }
    }
}

/// Saves auction and solutions to separate JSON files in the configured
/// directory. This function runs in a background task and logs errors without
/// failing the request.
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
/// This function waits 60 seconds before attempting to fetch, then retries up
/// to 10 times.
async fn fetch_and_save_competition_data(
    auction_id: crate::domain::auction::Id,
    cow_api_base_url: &str,
    save_dir: &std::path::Path,
) {
    use tokio::{
        fs,
        time::{Duration, sleep},
    };

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
                                    tracing::warn!(?err, "Failed to serialize competition data");
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

/// Verifies solutions against on-chain Balancer contracts and saves results
/// Accepts JSON solutions (possibly enhanced with liquidityDetails)
async fn verify_and_save_solutions(
    solutions_json: serde_json::Value,
    verifier: crate::infra::solution_verifier::SolutionVerifier,
    auction_id: crate::domain::auction::Id,
    save_dir: &std::path::Path,
) {
    use tokio::fs;

    let auction_id_num = match auction_id {
        crate::domain::auction::Id::Solve(id) => id,
        crate::domain::auction::Id::Quote => {
            tracing::debug!("Skipping verification for quote auction");
            return;
        }
    };

    // Extract solutions array from JSON
    let solutions_array = match solutions_json["solutions"].as_array() {
        Some(arr) => arr,
        None => {
            tracing::warn!("Solutions JSON missing 'solutions' array");
            return;
        }
    };

    tracing::info!(
        auction_id = auction_id_num,
        solutions_count = solutions_array.len(),
        has_liquidity_details = solutions_array
            .get(0)
            .and_then(|s| s["interactions"].as_array())
            .and_then(|i| i.get(0))
            .and_then(|i| i.get("liquidityDetails"))
            .is_some(),
        "Starting solution verification with enhanced liquidity data"
    );

    // Verify each solution in parallel
    let mut verification_futures = Vec::new();
    for (idx, solution) in solutions_array.iter().enumerate() {
        let verifier_clone = verifier.clone();
        let solution = solution.clone();
        verification_futures.push(tokio::spawn(async move {
            verifier_clone.verify_solution(&solution, idx).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(verification_futures)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();

    // Save results
    let filename = format!("{}_solution_verification.json", auction_id_num);
    let file_path = save_dir.join(filename);

    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(?err, "Failed to create verification directory");
        return;
    }

    let json_string = match serde_json::to_string_pretty(&results) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize verification results");
            return;
        }
    };

    match fs::write(&file_path, json_string).await {
        Ok(_) => {
            tracing::info!(
                auction_id = auction_id_num,
                file_path = ?file_path,
                solutions_verified = results.len(),
                "💾 Saved solution verification results"
            );
        }
        Err(err) => {
            tracing::warn!(?err, "Failed to write verification file");
        }
    }
}

/// Verifies swap logs against on-chain contract calls and saves the results.
async fn verify_and_save_swap_log(
    swap_records: Vec<crate::boundary::swap_logger::SwapRecord>,
    auction_id: Option<i64>,
    verifier: crate::infra::solution_verifier::SolutionVerifier,
    save_dir: &std::path::Path,
) {
    use tokio::fs;

    // Load liquidity file to get pool data including rates
    let liquidity_map = if let Some(id) = auction_id {
        let liquidity_file = save_dir.join(format!("{}_liquidity.json", id));
        match fs::read_to_string(&liquidity_file).await {
            Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(liq_json) => {
                    let mut map = std::collections::HashMap::new();
                    if let Some(liquidity_array) = liq_json["liquidity"].as_array() {
                        for pool in liquidity_array {
                            if let Some(pool_id) = pool["id"].as_str() {
                                map.insert(pool_id.to_string(), pool.clone());
                            }
                        }
                    }
                    map
                }
                Err(err) => {
                    tracing::warn!(?err, "Failed to parse liquidity JSON");
                    std::collections::HashMap::new()
                }
            },
            Err(err) => {
                tracing::debug!(
                    ?err,
                    "Could not read liquidity file for swap log verification"
                );
                std::collections::HashMap::new()
            }
        }
    } else {
        std::collections::HashMap::new()
    };

    // Enrich swap records with pool data from liquidity file
    let enriched_swaps: Vec<serde_json::Value> = swap_records
        .into_iter()
        .map(|swap| {
            let mut swap_json = serde_json::to_value(&swap).unwrap_or_default();

            if let Some(pool_data) = liquidity_map.get(&swap.liquidity_id) {
                // Add balancerPoolId to pool_params if present
                if let Some(balancer_pool_id) = pool_data["balancerPoolId"].as_str() {
                    if let Some(params) = swap_json["pool_params"].as_object_mut() {
                        params.insert(
                            "balancerPoolId".to_string(),
                            serde_json::Value::String(balancer_pool_id.to_string()),
                        );
                    }
                }

                // Determine pool version based on balancerPoolId presence
                let pool_version = if pool_data["balancerPoolId"].is_null() {
                    "V3"
                } else {
                    let pool_id = pool_data["balancerPoolId"].as_str().unwrap_or("");
                    if pool_id.len() > 42 { "V2" } else { "V3" }
                };
                swap_json["pool_version"] = serde_json::Value::String(pool_version.to_string());

                // Extract rate information for input and output tokens
                // Tokens can be either an object (dict) or array depending on format
                if let Some(tokens_obj) = pool_data["tokens"].as_object() {
                    let input_token = swap.input_token.to_lowercase();
                    let output_token = swap.output_token.to_lowercase();

                    // Tokens stored as {address: {rate, scalingFactor, ...}}
                    let token_in_data = tokens_obj.get(&input_token)
                        .or_else(|| tokens_obj.get(&swap.input_token));
                    let token_out_data = tokens_obj.get(&output_token)
                        .or_else(|| tokens_obj.get(&swap.output_token));

                    let token_in_rate = token_in_data.and_then(|t| t["rate"].as_str()).map(|s| s.to_string());
                    let token_in_rate_provider = token_in_data.and_then(|t| t["rateProvider"].as_str()).map(|s| s.to_string());
                    let token_in_scaling_factor = token_in_data.and_then(|t| t["scalingFactor"].as_str()).map(|s| s.to_string());

                    let token_out_rate = token_out_data.and_then(|t| t["rate"].as_str()).map(|s| s.to_string());
                    let token_out_rate_provider = token_out_data.and_then(|t| t["rateProvider"].as_str()).map(|s| s.to_string());
                    let token_out_scaling_factor = token_out_data.and_then(|t| t["scalingFactor"].as_str()).map(|s| s.to_string());

                    // Add rate info if we found any rate data
                    if token_in_rate.is_some() || token_out_rate.is_some() {
                        swap_json["rate_info"] = serde_json::json!({
                            "token_in_rate": token_in_rate.unwrap_or_else(|| "".to_string()),
                            "token_out_rate": token_out_rate.unwrap_or_else(|| "".to_string()),
                            "token_in_rate_provider": token_in_rate_provider.unwrap_or_else(|| "".to_string()),
                            "token_out_rate_provider": token_out_rate_provider.unwrap_or_else(|| "".to_string()),
                            "token_in_scaling_factor": token_in_scaling_factor.unwrap_or_else(|| "".to_string()),
                            "token_out_scaling_factor": token_out_scaling_factor.unwrap_or_else(|| "".to_string()),
                        });
                    }
                }
            }

            swap_json
        })
        .collect();

    // Add debug summary statistics
    let debug_stats = {
        let mut stats = std::collections::HashMap::new();
        for swap in &enriched_swaps {
            if swap["input_amount"].as_str() == Some("0") {
                let kind = swap["kind"].as_str().unwrap_or("unknown").to_string();
                let counter = stats.entry(kind).or_insert(0);
                *counter += 1;
            }
        }
        stats
    };

    // Convert swap records to JSON format expected by verifier
    let swap_log_json = serde_json::json!({
        "auction_id": auction_id,
        "swaps_count": enriched_swaps.len(),
        "debug_summary": {
            "zero_input_swaps_by_kind": debug_stats,
        },
        "swaps": enriched_swaps,
    });

    // Verify swap logs
    let verification_result = verifier.verify_swap_logs(&swap_log_json).await;

    // Determine filename
    let filename = match auction_id {
        Some(id) => format!("{}_swap_log_verification.json", id),
        None => {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            format!("quote_{}_swap_log_verification.json", timestamp)
        }
    };
    let file_path = save_dir.join(filename);

    // Create directory if needed
    if let Err(err) = fs::create_dir_all(save_dir).await {
        tracing::warn!(?err, directory = ?save_dir, "Failed to create directory");
        return;
    }

    // Serialize to pretty JSON
    let json_string = match serde_json::to_string_pretty(&verification_result) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(?err, "Failed to serialize swap log verification");
            return;
        }
    };

    // Write to file
    match fs::write(&file_path, json_string).await {
        Ok(_) => {
            tracing::info!(
                auction_id = ?auction_id,
                file_path = ?file_path,
                swaps_verified = verification_result.verified,
                swaps_failed = verification_result.failed,
                total_swaps = verification_result.total_swaps,
                "✅ Saved swap log verification results"
            );
        }
        Err(err) => {
            tracing::warn!(?err, "Failed to write swap log verification file");
        }
    }
}

/// Saves enhanced solutions (already created) to a JSON file
async fn save_enhanced_solutions_json(
    enhanced: serde_json::Value,
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
