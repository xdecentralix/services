use {
    contracts::{BalancerV2Vault, BalancerV3BatchRouter},
    ethcontract::{Address, Bytes, H160, U256},
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
}

#[derive(Debug, Serialize, Deserialize)]
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
        let (pool_address_opt, balancer_pool_id_opt, pool_kind_opt) =
            if let Some(details) = pool_details {
                (
                    details["address"].as_str(),
                    details["balancerPoolId"].as_str(),
                    details["kind"].as_str(),
                )
            } else {
                (None, None, None)
            };

        // Determine pool version ONLY by ID length (not by pool kind)
        // Prefer balancerPoolId from liquidityDetails, fall back to pool_id
        let pool_version = Self::detect_pool_version(balancer_pool_id_opt.unwrap_or(pool_id));

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

        let (quoted_amount_out, difference_bps, quote_error) = match quoted_amount {
            Ok(quote) => {
                let diff = calculate_difference_bps(&output_amount, &quote);
                (Some(quote), diff, None)
            }
            Err(e) => (None, None, Some(e.to_string())),
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
        }
    }

    /// Quote V2 swap via Vault.queryBatchSwap
    async fn quote_v2_swap(
        &self,
        balancer_pool_id: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<String, Box<dyn std::error::Error>> {
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
            assets,
            (
                H160::zero(), // sender (not needed for query)
                false,        // fromInternalBalance
                H160::zero(), // recipient (not needed for query)
                false,        // toInternalBalance
            ),
        );

        // Call the query (static call)
        let deltas = swap.call().await?;

        // Parse output: assetDeltas[1] should be positive (amount out)
        if deltas.len() < 2 {
            return Err("Invalid deltas returned from queryBatchSwap".into());
        }

        // Convert I256 to U256 (take absolute value since output is positive)
        let amount_out = if deltas[1].is_negative() {
            return Err("Expected positive output delta".into());
        } else {
            deltas[1].into_raw()
        };

        Ok(amount_out.to_string())
    }

    /// Quote V3 swap via Batch Router.querySwapExactIn
    async fn quote_v3_swap(
        &self,
        pool_address_str: &str,
        input_token: H160,
        output_token: H160,
        input_amount: U256,
    ) -> Result<String, Box<dyn std::error::Error>> {
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
        let query = self.batch_router.methods().query_swap_exact_in(
            vec![path],
            H160::zero(),  // sender (not needed for query)
            Bytes(vec![]), // empty userData
        );

        let (path_amounts_out, _tokens_out, _amounts_out) = query.call().await?;

        // Get the first path's output amount
        if path_amounts_out.is_empty() {
            return Err("No output amounts returned from querySwapExactIn".into());
        }

        Ok(path_amounts_out[0].to_string())
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
