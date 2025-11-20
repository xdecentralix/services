use {
    alloy::primitives,
    contracts::alloy::{
        BalancerV2Vault::{self, IVault},
        BalancerV3BatchRouter::{
            self,
            IBatchRouter::{SwapPathExactAmountIn, SwapPathStep},
        },
    },
    ethcontract::{Address, H160, U256},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    serde::{Deserialize, Serialize},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResult {
    pub solution_index: usize,
    pub swaps: Vec<SwapVerification>,
    pub total_gas_estimate: Option<u64>,
    pub verification_timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SwapLogVerificationResult {
    pub auction_id: Option<i64>,
    pub total_swaps: usize,
    pub verified: usize,
    pub failed: usize,
    pub by_pool_type: std::collections::HashMap<String, PoolTypeStats>,
    pub swaps: Vec<SwapLogVerification>,
    pub verification_timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolTypeStats {
    pub total: usize,
    pub verified: usize,
    pub perfect_matches: usize,
    pub within_1bps: usize,
    pub within_10bps: usize,
    pub errors: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SwapLogVerification {
    pub liquidity_id: String,
    pub kind: String,
    pub pool_address: String,
    pub pool_version: Option<String>,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: String,
    pub expected_amount_out: Option<String>,
    pub quoted_amount_out: Option<String>,
    pub difference_bps: Option<i64>,
    pub verified: bool,
    pub error: Option<String>,
    pub rate_info: Option<RateInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateInfo {
    pub token_in_rate: String,
    pub token_out_rate: String,
    pub token_in_rate_provider: String,
    pub token_out_rate_provider: String,
    pub token_in_scaling_factor: String,
    pub token_out_scaling_factor: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SwapVerification {
    pub interaction_index: usize,
    pub pool_id: String,
    pub pool_version: PoolVersion,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: String,
    pub expected_amount_out: String,
    pub quoted_amount_out: Option<String>,
    pub difference_bps: Option<i64>,
    pub quote_error: Option<String>,
    pub contract_call: Option<ContractCallDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractCallDetails {
    pub contract_address: String,
    pub contract_name: String,
    pub function_name: String,
    pub calldata: String,
    pub decoded_params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum PoolVersion {
    V2,
    V3,
}

#[derive(Clone)]
pub struct SolutionVerifier {
    vault: BalancerV2Vault::Instance,
    batch_router: BalancerV3BatchRouter::Instance,
}

impl SolutionVerifier {
    pub fn new(
        vault: BalancerV2Vault::Instance,
        batch_router: BalancerV3BatchRouter::Instance,
    ) -> Self {
        Self {
            vault,
            batch_router,
        }
    }

    /// Detect pool version by ID length
    fn detect_pool_version(pool_id: &str) -> PoolVersion {
        // V2 pool IDs are 66 chars (0x + 64 hex chars)
        // V3 pool IDs are 42 chars (same as address: 0x + 40 hex chars)
        if pool_id.len() > 42 {
            PoolVersion::V2
        } else {
            PoolVersion::V3
        }
    }

    /// Verify a single solution (accepts JSON to support enhanced solutions)
    pub async fn verify_solution(
        &self,
        solution: &serde_json::Value,
        solution_index: usize,
    ) -> VerificationResult {
        let mut swaps = Vec::new();

        if let Some(interactions) = solution["interactions"].as_array() {
            for (idx, interaction) in interactions.iter().enumerate() {
                if interaction["kind"] == "liquidity" {
                    let verification = self.verify_swap(interaction, idx).await;
                    swaps.push(verification);
                }
            }
        }

        VerificationResult {
            solution_index,
            swaps,
            total_gas_estimate: None,
            verification_timestamp: chrono::Utc::now().timestamp() as u64,
        }
    }

    /// Verify swap logs from the swap logger
    pub async fn verify_swap_logs(
        &self,
        swap_log_json: &serde_json::Value,
    ) -> SwapLogVerificationResult {
        let auction_id = swap_log_json["auction_id"].as_i64();
        let swaps_array = match swap_log_json["swaps"].as_array() {
            Some(arr) => arr,
            None => {
                return SwapLogVerificationResult {
                    auction_id,
                    total_swaps: 0,
                    verified: 0,
                    failed: 0,
                    by_pool_type: std::collections::HashMap::new(),
                    swaps: vec![],
                    verification_timestamp: chrono::Utc::now().timestamp() as u64,
                };
            }
        };

        let mut verifications = Vec::new();
        let mut pool_type_stats: std::collections::HashMap<String, PoolTypeStats> =
            std::collections::HashMap::new();

        for swap in swaps_array {
            let verification = self.verify_swap_log_entry(swap).await;

            // Update pool type stats
            let kind = verification.kind.clone();
            let stats = pool_type_stats.entry(kind).or_insert(PoolTypeStats {
                total: 0,
                verified: 0,
                perfect_matches: 0,
                within_1bps: 0,
                within_10bps: 0,
                errors: 0,
            });

            stats.total += 1;
            if verification.verified {
                stats.verified += 1;
                if let Some(diff_bps) = verification.difference_bps {
                    let abs_diff = diff_bps.abs();
                    if abs_diff == 0 {
                        stats.perfect_matches += 1;
                    } else if abs_diff <= 1 {
                        stats.within_1bps += 1;
                    } else if abs_diff <= 10 {
                        stats.within_10bps += 1;
                    }
                }
            } else {
                stats.errors += 1;
            }

            verifications.push(verification);
        }

        let verified_count = verifications.iter().filter(|v| v.verified).count();
        let failed_count = verifications.len() - verified_count;

        SwapLogVerificationResult {
            auction_id,
            total_swaps: verifications.len(),
            verified: verified_count,
            failed: failed_count,
            by_pool_type: pool_type_stats,
            swaps: verifications,
            verification_timestamp: chrono::Utc::now().timestamp() as u64,
        }
    }

    /// Verify a single swap log entry
    async fn verify_swap_log_entry(&self, swap: &serde_json::Value) -> SwapLogVerification {
        let liquidity_id = swap["liquidity_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let kind = swap["kind"].as_str().unwrap_or("unknown").to_string();
        let pool_address = swap["address"].as_str().unwrap_or("").to_string();
        let input_token_str = swap["input_token"].as_str().unwrap_or("");
        let output_token_str = swap["output_token"].as_str().unwrap_or("");
        let input_amount_str = swap["input_amount"].as_str().unwrap_or("0");
        let expected_output = swap["output_amount"].as_str().map(|s| s.to_string());

        // Parse addresses
        let input_token: Address = input_token_str.parse().unwrap_or_default();
        let output_token: Address = output_token_str.parse().unwrap_or_default();
        let input_amount = U256::from_dec_str(input_amount_str).unwrap_or_default();

        // Extract rate information if present
        let rate_info = swap.get("rate_info").and_then(|ri| {
            Some(RateInfo {
                token_in_rate: ri["token_in_rate"].as_str().unwrap_or("").to_string(),
                token_out_rate: ri["token_out_rate"].as_str().unwrap_or("").to_string(),
                token_in_rate_provider: ri["token_in_rate_provider"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                token_out_rate_provider: ri["token_out_rate_provider"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                token_in_scaling_factor: ri["token_in_scaling_factor"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                token_out_scaling_factor: ri["token_out_scaling_factor"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            })
        });

        // Extract pool version if present
        let pool_version = swap.get("pool_version").and_then(|v| v.as_str()).map(|s| s.to_string());

        // Skip if the swap failed in the solver (output_amount is null)
        if expected_output.is_none() {
            return SwapLogVerification {
                liquidity_id,
                kind,
                pool_address: pool_address.clone(),
                pool_version,
                token_in: input_token,
                token_out: output_token,
                amount_in: input_amount_str.to_string(),
                expected_amount_out: None,
                quoted_amount_out: None,
                difference_bps: None,
                verified: false,
                error: Some("Swap failed in solver (no output calculated)".to_string()),
                rate_info,
            };
        }

        // Try to extract pool details
        let pool_params = swap.get("pool_params");
        let balancer_pool_id = pool_params
            .and_then(|p| p.get("balancerPoolId"))
            .and_then(|id| id.as_str());

        // Detect pool version: if no balancerPoolId, it's V3
        let pool_version_enum = if balancer_pool_id.is_none() {
            PoolVersion::V3
        } else {
            Self::detect_pool_version(balancer_pool_id.unwrap())
        };

        // Convert enum to string, preferring the one from swap data if available
        let pool_version_str = pool_version.or_else(|| {
            Some(match pool_version_enum {
                PoolVersion::V2 => "V2".to_string(),
                PoolVersion::V3 => "V3".to_string(),
            })
        });

        // Quote the swap
        let quote_result = match pool_version_enum {
            PoolVersion::V2 => {
                if let Some(pool_id) = balancer_pool_id {
                    self.quote_v2_swap(
                        pool_id,
                        H160::from(input_token.0),
                        H160::from(output_token.0),
                        input_amount,
                    )
                    .await
                } else {
                    Err("Missing balancerPoolId for V2 pool".into())
                }
            }
            PoolVersion::V3 => {
                if !pool_address.is_empty() {
                    self.quote_v3_swap(
                        &pool_address,
                        H160::from(input_token.0),
                        H160::from(output_token.0),
                        input_amount,
                    )
                    .await
                } else {
                    Err("Missing pool address for V3 pool".into())
                }
            }
        };

        match quote_result {
            Ok((quoted_amount, _call_details)) => {
                let expected_u256 =
                    U256::from_dec_str(expected_output.as_ref().unwrap()).unwrap_or_default();
                let diff_bps = calculate_difference_bps(&expected_u256, &quoted_amount);

                SwapLogVerification {
                    liquidity_id,
                    kind,
                    pool_address,
                    pool_version: pool_version_str.clone(),
                    token_in: input_token,
                    token_out: output_token,
                    amount_in: input_amount_str.to_string(),
                    expected_amount_out: expected_output,
                    quoted_amount_out: Some(quoted_amount),
                    difference_bps: diff_bps,
                    verified: true,
                    error: None,
                    rate_info: rate_info.clone(),
                }
            }
            Err(e) => SwapLogVerification {
                liquidity_id,
                kind,
                pool_address,
                pool_version: pool_version_str,
                token_in: input_token,
                token_out: output_token,
                amount_in: input_amount_str.to_string(),
                expected_amount_out: expected_output,
                quoted_amount_out: None,
                difference_bps: None,
                verified: false,
                error: Some(e.to_string()),
                rate_info,
            },
        }
    }

    /// Verify a single swap interaction (accepts JSON to support enhanced
    /// solutions)
    async fn verify_swap(
        &self,
        interaction: &serde_json::Value,
        interaction_index: usize,
    ) -> SwapVerification {
        // Extract basic fields
        let pool_id = interaction["id"].as_str().unwrap_or("unknown");
        let input_token_str = interaction["inputToken"].as_str().unwrap_or("");
        let output_token_str = interaction["outputToken"].as_str().unwrap_or("");
        let input_amount_str = interaction["inputAmount"].as_str().unwrap_or("0");
        let output_amount_str = interaction["outputAmount"].as_str().unwrap_or("0");

        // Parse token addresses
        let input_token: Address = input_token_str.parse().unwrap_or_default();
        let output_token: Address = output_token_str.parse().unwrap_or_default();
        let input_amount = U256::from_dec_str(input_amount_str).unwrap_or_default();
        let output_amount = U256::from_dec_str(output_amount_str).unwrap_or_default();

        // Try to extract liquidityDetails (enhanced solutions)
        let pool_details = interaction.get("liquidityDetails");

        // Extract pool address and Balancer pool ID from liquidityDetails if available
        let (pool_address_opt, balancer_pool_id_opt) = if let Some(details) = pool_details {
            (
                details["address"].as_str(),
                details["balancerPoolId"].as_str(),
            )
        } else {
            (None, None)
        };

        // Determine pool version:
        // - If balancerPoolId is None/null, it's a V3 pool (V3 pools don't have
        //   V2-style pool IDs)
        // - If balancerPoolId exists, detect version by ID length
        let pool_version = if balancer_pool_id_opt.is_none() {
            PoolVersion::V3
        } else {
            Self::detect_pool_version(balancer_pool_id_opt.unwrap_or(pool_id))
        };

        // Quote using appropriate method with enhanced data
        let quoted_amount = match pool_version {
            PoolVersion::V2 => {
                if let Some(pool_id_hex) = balancer_pool_id_opt {
                    self.quote_v2_swap(
                        pool_id_hex,
                        H160::from(input_token.0),
                        H160::from(output_token.0),
                        input_amount,
                    )
                    .await
                } else {
                    Err("Missing balancerPoolId for V2 pool in liquidityDetails".into())
                }
            }
            PoolVersion::V3 => {
                if let Some(address) = pool_address_opt {
                    self.quote_v3_swap(
                        address,
                        H160::from(input_token.0),
                        H160::from(output_token.0),
                        input_amount,
                    )
                    .await
                } else {
                    Err("Missing pool address for V3 pool in liquidityDetails".into())
                }
            }
        };

        let (quoted_amount_out, difference_bps, quote_error, contract_call) = match quoted_amount {
            Ok((quote, call_details)) => {
                let diff = calculate_difference_bps(&output_amount, &quote);
                (Some(quote), diff, None, Some(call_details))
            }
            Err(e) => {
                // For V3 calls, we still want to save the call details even on error
                // so the user can see what was actually attempted
                let error_call_details = if pool_version == PoolVersion::V3 {
                    if let Some(address) = pool_address_opt {
                        Some(create_v3_call_details(
                            &self.batch_router,
                            address,
                            &input_token,
                            &output_token,
                            input_amount,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                (None, None, Some(e.to_string()), error_call_details)
            }
        };

        SwapVerification {
            interaction_index,
            pool_id: pool_id.to_string(),
            pool_version,
            token_in: input_token,
            token_out: output_token,
            amount_in: input_amount.to_string(),
            expected_amount_out: output_amount.to_string(),
            quoted_amount_out,
            difference_bps,
            quote_error,
            contract_call,
        }
    }

    /// Quote V2 swap via Vault.queryBatchSwap
    /// This uses a static call (eth_call) to query the expected output amount.
    async fn quote_v2_swap(
        &self,
        balancer_pool_id: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<(String, ContractCallDetails), Box<dyn std::error::Error>> {
        // Parse pool ID (it's a hex string starting with 0x)
        let pool_id_bytes = if balancer_pool_id.starts_with("0x") {
            const_hex::decode(&balancer_pool_id[2..])?
        } else {
            const_hex::decode(balancer_pool_id)?
        };

        if pool_id_bytes.len() != 32 {
            return Err(format!("Invalid V2 pool ID length: {}", pool_id_bytes.len()).into());
        }

        let mut pool_id = [0u8; 32];
        pool_id.copy_from_slice(&pool_id_bytes);

        // Build assets array using alloy types
        let assets = vec![input_token.into_alloy(), output_token.into_alloy()];

        // Create BatchSwapStep using alloy types
        let swap_step = IVault::BatchSwapStep {
            poolId: primitives::FixedBytes::from(pool_id),
            assetInIndex: primitives::U256::from(0u64),
            assetOutIndex: primitives::U256::from(1u64),
            amount: input_amount.into_alloy(),
            userData: primitives::Bytes::new(),
        };

        // Create FundManagement struct
        let funds = IVault::FundManagement {
            sender: primitives::Address::ZERO,
            fromInternalBalance: false,
            recipient: primitives::Address::ZERO,
            toInternalBalance: false,
        };

        // Build the call - .call() automatically makes it a static call (eth_call)
        let call_builder = self.vault.queryBatchSwap(
            0u8, // SwapKind.GIVEN_IN
            vec![swap_step.clone()],
            assets.clone(),
            funds,
        );

        // Capture contract call details for debugging
        let calldata = format!("0x{}", const_hex::encode(call_builder.calldata()));

        let decoded_params = serde_json::json!({
            "kind": "GIVEN_IN (0)",
            "swaps": [{
                "poolId": balancer_pool_id,
                "assetInIndex": 0,
                "assetOutIndex": 1,
                "amount": input_amount.to_string(),
                "userData": "0x"
            }],
            "assets": vec![
                format!("{:?}", input_token),
                format!("{:?}", output_token)
            ],
            "funds": {
                "sender": "0x0000000000000000000000000000000000000000",
                "fromInternalBalance": false,
                "recipient": "0x0000000000000000000000000000000000000000",
                "toInternalBalance": false
            }
        });

        let call_details = ContractCallDetails {
            contract_address: format!("{:?}", self.vault.address().into_legacy()),
            contract_name: "BalancerV2Vault".to_string(),
            function_name: "queryBatchSwap".to_string(),
            calldata,
            decoded_params,
        };

        // Execute the static call
        let result = call_builder.call().await;

        match result {
            Ok(deltas) => {
                // Parse output: assetDeltas[1] represents net token flow for output token
                // In Balancer V2:
                //   - Positive delta = tokens going INTO vault (user sends)
                //   - Negative delta = tokens coming OUT of vault (user receives)
                // For the output token in a swap, we expect a NEGATIVE delta
                if deltas.len() < 2 {
                    return Err("Invalid deltas returned from queryBatchSwap".into());
                }

                let delta_out = deltas[1];

                // Check if the signed value is negative
                let amount_out = if delta_out.is_negative() {
                    // Negative means tokens out - convert to unsigned by negating
                    // For Signed types, we need to negate and convert to unsigned
                    let abs_value = delta_out.abs();
                    primitives::U256::from_limbs(abs_value.into_limbs())
                } else {
                    // Positive means tokens in, which is wrong for output token
                    return Err("Expected negative output delta (tokens out of vault)".into());
                };

                Ok((amount_out.to_string(), call_details))
            }
            Err(e) => {
                // Return the error - call details will be saved separately in the JSON
                Err(format!("Query failed: {:?}", e).into())
            }
        }
    }

    /// Quote V3 swap via Batch Router.querySwapExactIn
    /// This uses a static call (eth_call) to query the expected output amount.
    async fn quote_v3_swap(
        &self,
        pool_address_str: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<(String, ContractCallDetails), Box<dyn std::error::Error>> {
        // Parse pool address from string
        let pool_address: H160 = pool_address_str.parse()?;

        // Build SwapPathExactAmountIn using alloy types
        let path = SwapPathExactAmountIn {
            tokenIn: input_token.into_alloy(),
            steps: vec![SwapPathStep {
                pool: pool_address.into_alloy(),
                tokenOut: output_token.into_alloy(),
                isBuffer: false,
            }],
            exactAmountIn: input_amount.into_alloy(),
            minAmountOut: primitives::U256::ZERO,
        };

        // Build the call - .call() automatically makes it a static call (eth_call)
        let call_builder = self.batch_router.querySwapExactIn(
            vec![path.clone()],
            *self.batch_router.address(), // sender (required for pools with hooks)
            primitives::Bytes::new(),     // empty userData
        );

        // Capture contract call details for debugging
        let calldata = format!("0x{}", const_hex::encode(call_builder.calldata()));

        let decoded_params = serde_json::json!({
            "paths": [{
                "tokenIn": format!("{:?}", input_token),
                "steps": [{
                    "pool": pool_address_str,
                    "tokenOut": format!("{:?}", output_token),
                    "isBuffer": false
                }],
                "exactAmountIn": input_amount.to_string(),
                "minAmountOut": "0"
            }],
            "sender": format!("{:?}", self.batch_router.address().into_legacy()),
            "userData": "0x"
        });

        let call_details = ContractCallDetails {
            contract_address: format!("{:?}", self.batch_router.address().into_legacy()),
            contract_name: "BalancerV3BatchRouter".to_string(),
            function_name: "querySwapExactIn".to_string(),
            calldata,
            decoded_params,
        };

        // Execute the static call
        let result = call_builder.call().await;

        match result {
            Ok(return_data) => {
                let path_amounts_out = return_data.pathAmountsOut;
                // Get the first path's output amount
                if path_amounts_out.is_empty() {
                    return Err("No output amounts returned from querySwapExactIn".into());
                }
                Ok((path_amounts_out[0].to_string(), call_details))
            }
            Err(e) => {
                // Return the error - call details will be saved separately in the JSON
                Err(format!("Query failed: {:?}", e).into())
            }
        }
    }
}

fn create_v3_call_details(
    batch_router: &BalancerV3BatchRouter::Instance,
    pool_address: &str,
    input_token: &Address,
    output_token: &Address,
    input_amount: U256,
) -> ContractCallDetails {
    let decoded_params = serde_json::json!({
        "paths": [{
            "tokenIn": format!("{:?}", H160::from(input_token.0)),
            "steps": [{
                "pool": pool_address,
                "tokenOut": format!("{:?}", H160::from(output_token.0)),
                "isBuffer": false
            }],
            "exactAmountIn": input_amount.to_string(),
            "minAmountOut": "0"
        }],
        "sender": format!("{:?}", batch_router.address().into_legacy()),
        "userData": "0x"
    });

    ContractCallDetails {
        contract_address: format!("{:?}", batch_router.address().into_legacy()),
        contract_name: "BalancerV3BatchRouter".to_string(),
        function_name: "querySwapExactIn".to_string(),
        calldata: "(error - call details captured without execution)".to_string(),
        decoded_params,
    }
}

fn calculate_difference_bps(expected: &U256, actual: &str) -> Option<i64> {
    // Parse actual amount
    let actual_u256 = U256::from_dec_str(actual).ok()?;

    // Calculate difference in basis points
    // diff_bps = ((actual - expected) / expected) * 10000
    if *expected == U256::zero() {
        return None;
    }

    let diff = if actual_u256 > *expected {
        let delta = actual_u256 - *expected;
        let bps = (delta * 10000u64) / *expected;
        bps.as_u64() as i64
    } else {
        let delta = *expected - actual_u256;
        let bps = (delta * 10000u64) / *expected;
        -(bps.as_u64() as i64)
    };

    Some(diff)
}
