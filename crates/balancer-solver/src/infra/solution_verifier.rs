use {
    contracts::{BalancerV2Vault, BalancerV3BatchRouter},
    ethcontract::{Account, Address, Bytes, H160, U256},
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
    vault: BalancerV2Vault,
    batch_router: BalancerV3BatchRouter,
}

impl SolutionVerifier {
    pub fn new(vault: BalancerV2Vault, batch_router: BalancerV3BatchRouter) -> Self {
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
    async fn quote_v2_swap(
        &self,
        balancer_pool_id: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<(String, ContractCallDetails), Box<dyn std::error::Error>> {
        // Parse pool ID (it's a hex string starting with 0x)
        let pool_id_bytes = if balancer_pool_id.starts_with("0x") {
            hex::decode(&balancer_pool_id[2..])?
        } else {
            hex::decode(balancer_pool_id)?
        };

        if pool_id_bytes.len() != 32 {
            return Err(format!("Invalid V2 pool ID length: {}", pool_id_bytes.len()).into());
        }

        let mut pool_id = [0u8; 32];
        pool_id.copy_from_slice(&pool_id_bytes);

        // Build assets array: [token_in, token_out]
        let assets = vec![input_token, output_token];

        // Create BatchSwapStep
        let swap = self.vault.methods().query_batch_swap(
            0u8.into(), // SwapKind.GIVEN_IN
            vec![(
                Bytes(pool_id),
                0u64.into(), // assetInIndex
                1u64.into(), // assetOutIndex
                input_amount,
                Bytes(vec![]), // empty userData
            )],
            assets.clone(),
            (
                H160::zero(), // sender (not needed for query)
                false,        // fromInternalBalance
                H160::zero(), // recipient (not needed for query)
                false,        // toInternalBalance
            ),
        );

        // Capture contract call details for debugging
        let calldata = swap
            .tx
            .data
            .clone()
            .map(|d| format!("0x{}", hex::encode(d.0)))
            .unwrap_or_else(|| "0x".to_string());

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
                format!("{:?}", assets[0]),
                format!("{:?}", assets[1])
            ],
            "funds": {
                "sender": "0x0000000000000000000000000000000000000000",
                "fromInternalBalance": false,
                "recipient": "0x0000000000000000000000000000000000000000",
                "toInternalBalance": false
            }
        });

        let call_details = ContractCallDetails {
            contract_address: format!("{:?}", self.vault.address()),
            contract_name: "BalancerV2Vault".to_string(),
            function_name: "queryBatchSwap".to_string(),
            calldata,
            decoded_params,
        };

        // Call the query (static call)
        let deltas = swap.call().await?;

        // Parse output: assetDeltas[1] represents net token flow for output token
        // In Balancer V2:
        //   - Positive delta = tokens going INTO vault (user sends)
        //   - Negative delta = tokens coming OUT of vault (user receives)
        // For the output token in a swap, we expect a NEGATIVE delta
        if deltas.len() < 2 {
            return Err("Invalid deltas returned from queryBatchSwap".into());
        }

        let amount_out = if deltas[1].is_negative() {
            // Negative means tokens out - negate to get positive amount
            (-deltas[1]).into_raw()
        } else {
            // Positive means tokens in, which is wrong for output token
            return Err("Expected negative output delta (tokens out of vault)".into());
        };

        Ok((amount_out.to_string(), call_details))
    }

    /// Quote V3 swap via Batch Router.querySwapExactIn
    async fn quote_v3_swap(
        &self,
        pool_address_str: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<(String, ContractCallDetails), Box<dyn std::error::Error>> {
        // Parse pool address from string
        let pool_address: H160 = pool_address_str.parse()?;

        // Build SwapPathExactAmountIn
        let path = (
            input_token, // tokenIn
            vec![(
                pool_address, // pool
                output_token, // tokenOut
                false,        // isBuffer
            )],
            input_amount, // exactAmountIn
            U256::zero(), // minAmountOut (no minimum for query)
        );

        // Call querySwapExactIn
        // IMPORTANT: Must set .from() to make this a proper staticcall
        let query = self.batch_router.methods().query_swap_exact_in(
            vec![path.clone()],
            self.batch_router.address(),  // sender (required for pools with hooks)
            Bytes(vec![]), // empty userData
        )
        .from(Account::Local(H160::zero(), None));  // Set from address for the eth_call

        // Capture contract call details for debugging
        let calldata = query
            .tx
            .data
            .clone()
            .map(|d| format!("0x{}", hex::encode(d.0)))
            .unwrap_or_else(|| "0x".to_string());

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
            "sender": format!("{:?}", self.batch_router.address()),
            "userData": "0x",
            "from": "0x0000000000000000000000000000000000000000"
        });

        let call_details = ContractCallDetails {
            contract_address: format!("{:?}", self.batch_router.address()),
            contract_name: "BalancerV3BatchRouter".to_string(),
            function_name: "querySwapExactIn".to_string(),
            calldata,
            decoded_params,
        };

        let result = query.call().await;

        match result {
            Ok((path_amounts_out, _tokens_out, _amounts_out)) => {
                // Get the first path's output amount
                if path_amounts_out.is_empty() {
                    return Err("No output amounts returned from querySwapExactIn".into());
                }
                Ok((path_amounts_out[0].to_string(), call_details))
            }
            Err(e) => {
                // Return the error - call details will be saved separately in the JSON
                Err(Box::new(e))
            }
        }
    }
}

fn create_v3_call_details(
    batch_router: &BalancerV3BatchRouter,
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
        "sender": format!("{:?}", batch_router.address()),
        "userData": "0x"
    });

    ContractCallDetails {
        contract_address: format!("{:?}", batch_router.address()),
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
