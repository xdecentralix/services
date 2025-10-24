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
    pub fn new(
        vault: BalancerV2Vault,
        batch_router: BalancerV3BatchRouter,
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

    /// Verify a single solution
    pub async fn verify_solution(
        &self,
        solution: &solvers_dto::solution::Solution,
        solution_index: usize,
    ) -> VerificationResult {
        let mut swaps = Vec::new();
        
        for (idx, interaction) in solution.interactions.iter().enumerate() {
            if let solvers_dto::solution::Interaction::Liquidity(liq) = interaction {
                let verification = self.verify_swap(liq, idx).await;
                swaps.push(verification);
            }
        }
        
        VerificationResult {
            solution_index,
            swaps,
            total_gas_estimate: None,
            verification_timestamp: chrono::Utc::now().timestamp() as u64,
        }
    }

    /// Verify a single swap interaction
    async fn verify_swap(
        &self,
        interaction: &solvers_dto::solution::LiquidityInteraction,
        interaction_index: usize,
    ) -> SwapVerification {
        let pool_version = Self::detect_pool_version(&interaction.id);
        
        let quoted_amount = match pool_version {
            PoolVersion::V2 => self.quote_v2_swap(interaction).await,
            PoolVersion::V3 => self.quote_v3_swap(interaction).await,
        };
        
        let (quoted_amount_out, difference_bps, quote_error) = match quoted_amount {
            Ok(quote) => {
                let diff = calculate_difference_bps(
                    &interaction.output_amount,
                    &quote,
                );
                (Some(quote), diff, None)
            }
            Err(e) => (None, None, Some(e.to_string())),
        };
        
        SwapVerification {
            interaction_index,
            pool_id: interaction.id.clone(),
            pool_version,
            token_in: interaction.input_token,
            token_out: interaction.output_token,
            amount_in: interaction.input_amount.to_string(),
            expected_amount_out: interaction.output_amount.to_string(),
            quoted_amount_out,
            difference_bps,
            quote_error,
        }
    }

    /// Quote V2 swap via Vault.queryBatchSwap
    async fn quote_v2_swap(
        &self,
        interaction: &solvers_dto::solution::LiquidityInteraction,
    ) -> Result<String, Box<dyn std::error::Error>> {
        // Parse pool ID (it's a hex string starting with 0x)
        let pool_id_bytes = if interaction.id.starts_with("0x") {
            hex::decode(&interaction.id[2..])?
        } else {
            hex::decode(&interaction.id)?
        };
        
        if pool_id_bytes.len() != 32 {
            return Err(format!("Invalid V2 pool ID length: {}", pool_id_bytes.len()).into());
        }
        
        let mut pool_id = [0u8; 32];
        pool_id.copy_from_slice(&pool_id_bytes);

        // Build assets array: [token_in, token_out]
        let assets = vec![
            H160::from(interaction.input_token.0),
            H160::from(interaction.output_token.0),
        ];

        // Create BatchSwapStep
        let swap = self.vault.methods().query_batch_swap(
            0u8.into(), // SwapKind.GIVEN_IN
            vec![(
                Bytes(pool_id),
                0u64.into(),  // assetInIndex
                1u64.into(),  // assetOutIndex
                interaction.input_amount,
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
        interaction: &solvers_dto::solution::LiquidityInteraction,
    ) -> Result<String, Box<dyn std::error::Error>> {
        // Parse pool address from ID
        let pool_address: H160 = interaction.id.parse()?;
        
        // Build SwapPathExactAmountIn
        let path = (
            H160::from(interaction.input_token.0), // tokenIn
            vec![
                (
                    pool_address,                               // pool
                    H160::from(interaction.output_token.0),    // tokenOut
                    false,                                      // isBuffer
                )
            ],
            interaction.input_amount,  // exactAmountIn
            U256::zero(),             // minAmountOut (no minimum for query)
        );
        
        // Call querySwapExactIn
        let query = self.batch_router.methods().query_swap_exact_in(
            vec![path],
            H160::zero(),    // sender (not needed for query)
            Bytes(vec![]),   // empty userData
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

