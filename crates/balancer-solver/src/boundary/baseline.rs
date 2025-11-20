//! Boundary wrappers around the [`shared`] Baseline solving logic.

use {
    crate::{
        boundary::{self, liquidity::erc4626 as boundary_erc4626, swap_logger},
        domain::{eth, liquidity, order, solver},
    },
    contracts::alloy::UniswapV3QuoterV2,
    ethereum_types::{H160, U256},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    model::TokenPair,
    shared::{
        baseline_solver::{self, BaseTokens, BaselineSolvable},
        ethrpc::Web3,
    },
    std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    },
};

pub struct Solver<'a> {
    base_tokens: BaseTokens,
    onchain_liquidity: HashMap<TokenPair, Vec<OnchainLiquidity>>,
    liquidity: HashMap<liquidity::Id, &'a liquidity::Liquidity>,
    swap_logger: Option<swap_logger::SwapLogger>,
}

impl<'a> Solver<'a> {
    pub fn new(
        weth: &eth::WethAddress,
        base_tokens: &HashSet<eth::TokenAddress>,
        liquidity: &'a [liquidity::Liquidity],
        uni_v3_quoter_v2: Option<Arc<UniswapV3QuoterV2::Instance>>,
        erc4626_web3: Option<&Web3>,
    ) -> Self {
        Self {
            base_tokens: to_boundary_base_tokens(weth, base_tokens),
            onchain_liquidity: to_boundary_liquidity(liquidity, uni_v3_quoter_v2, erc4626_web3),
            liquidity: liquidity
                .iter()
                .map(|liquidity| (liquidity.id.clone(), liquidity))
                .collect(),
            swap_logger: None,
        }
    }

    /// Enable swap logging for debugging and verification
    pub fn with_swap_logger(mut self, logger: swap_logger::SwapLogger) -> Self {
        self.swap_logger = Some(logger);
        self
    }

    pub async fn route(
        &self,
        request: solver::Request,
        max_hops: usize,
    ) -> Option<solver::Route<'a>> {
        let candidates = self.base_tokens.path_candidates_with_hops(
            request.sell.token.0,
            request.buy.token.0,
            max_hops,
        );

        let (segments, _) = match request.side {
            order::Side::Buy => {
                let futures = candidates.iter().map(|path| async {
                    let sell = baseline_solver::estimate_sell_amount(
                        request.buy.amount,
                        path,
                        &self.onchain_liquidity,
                    )
                    .await?;
                    let segments = self
                        .traverse_path(&sell.path, request.sell.token.0, sell.value)
                        .await?;

                    let buy = segments.last().map(|segment| segment.output.amount);
                    if buy.map(|buy| buy >= request.buy.amount) != Some(true) {
                        tracing::warn!(
                            ?request,
                            ?segments,
                            "invalid buy estimate does not cover order"
                        );
                        return None;
                    }

                    (sell.value <= request.sell.amount).then_some((segments, sell))
                });
                futures::future::join_all(futures)
                    .await
                    .into_iter()
                    .flatten()
                    .min_by_key(|(_, sell)| sell.value)?
            }
            order::Side::Sell => {
                let futures = candidates.iter().map(|path| async {
                    let buy = baseline_solver::estimate_buy_amount(
                        request.sell.amount,
                        path,
                        &self.onchain_liquidity,
                    )
                    .await?;
                    let segments = self
                        .traverse_path(&buy.path, request.sell.token.0, request.sell.amount)
                        .await?;

                    let sell = segments.first().map(|segment| segment.input.amount);
                    if sell.map(|sell| sell >= request.sell.amount) != Some(true) {
                        tracing::warn!(
                            ?request,
                            ?segments,
                            "invalid sell estimate does not cover order"
                        );
                        return None;
                    }

                    (buy.value >= request.buy.amount).then_some((segments, buy))
                });
                futures::future::join_all(futures)
                    .await
                    .into_iter()
                    .flatten()
                    .max_by_key(|(_, buy)| buy.value)?
            }
        };

        solver::Route::new(segments)
    }

    async fn traverse_path(
        &self,
        path: &[&OnchainLiquidity],
        mut sell_token: H160,
        mut sell_amount: U256,
    ) -> Option<Vec<solver::Segment<'a>>> {
        let mut segments = Vec::new();
        for liquidity in path {
            let reference_liquidity = self
                .liquidity
                .get(&liquidity.id)
                .expect("boundary liquidity does not match ID");

            let buy_token = liquidity
                .token_pair
                .other(&sell_token.into_alloy())
                .expect("Inconsistent path");

            // Log the swap attempt if logging is enabled
            let buy_amount = if let Some(ref logger) = self.swap_logger {
                // Configure which pool types to log (set to log all by default)
                let should_log = matches!(
                    liquidity.kind_str(),
                    "weightedProduct"
                        | "stable"
                        | "gyroE"
                        | "gyro2CLP"
                        | "gyro3CLP"
                        | "reClamm"
                        | "quantAmm"
                        | "erc4626"
                );

                let result = liquidity
                    .get_amount_out(buy_token.into_legacy(), (sell_amount, sell_token))
                    .await;

                // Only log if this is a pool type we're interested in
                if should_log {
                    // Build debug metadata for problematic swaps
                    let debug = if sell_amount.is_zero()
                        || result.is_none()
                        || (result.is_some() && result.as_ref().unwrap().is_zero())
                    {
                        let mut note = Vec::new();
                        if sell_amount.is_zero() {
                            note.push(
                                "Input amount is zero - likely from failed previous hop or \
                                 pathfinding issue",
                            );
                        }
                        if result.is_none() {
                            note.push("get_amount_out returned None - swap calculation failed");
                        } else if result.is_some() && result.as_ref().unwrap().is_zero() {
                            note.push(
                                "Output is zero despite calculation succeeding - check math \
                                 implementation",
                            );
                        }

                        Some(swap_logger::DebugMetadata {
                            zero_input: if sell_amount.is_zero() {
                                Some(true)
                            } else {
                                None
                            },
                            path_info: Some(format!(
                                "Segment in path, sell_token: {:#x}, buy_token: {:#x}",
                                sell_token,
                                buy_token.into_legacy()
                            )),
                            note: if note.is_empty() {
                                None
                            } else {
                                Some(note.join("; "))
                            },
                        })
                    } else {
                        None
                    };

                    logger.log_swap(swap_logger::SwapRecord {
                        liquidity_id: liquidity.id.0.clone(),
                        kind: liquidity.kind_str().to_string(),
                        address: format!("{:#x}", liquidity.address()),
                        input_token: format!("{:#x}", sell_token),
                        input_amount: sell_amount.to_string(),
                        output_token: format!("{:#x}", buy_token.into_legacy()),
                        output_amount: result.as_ref().map(|amt| amt.to_string()),
                        pool_params: extract_pool_params(reference_liquidity),
                        debug,
                    });
                }

                result?
            } else {
                liquidity
                    .get_amount_out(buy_token.into_legacy(), (sell_amount, sell_token))
                    .await?
            };

            segments.push(solver::Segment {
                liquidity: reference_liquidity,
                input: eth::Asset {
                    token: eth::TokenAddress(sell_token),
                    amount: sell_amount,
                },
                output: eth::Asset {
                    token: eth::TokenAddress(buy_token.into_legacy()),
                    amount: buy_amount,
                },
                gas: eth::Gas(liquidity.gas_cost().await.into()),
            });

            sell_token = buy_token.into_legacy();
            sell_amount = buy_amount;
        }
        Some(segments)
    }
}

fn to_boundary_liquidity(
    liquidity: &[liquidity::Liquidity],
    uni_v3_quoter_v2: Option<Arc<contracts::alloy::UniswapV3QuoterV2::Instance>>,
    erc4626_web3: Option<&Web3>,
) -> HashMap<TokenPair, Vec<OnchainLiquidity>> {
    liquidity
        .iter()
        .fold(HashMap::new(), |mut onchain_liquidity, liquidity| {
            match &liquidity.state {
                liquidity::State::ConstantProduct(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::constant_product::to_boundary_pool(
                            liquidity.address,
                            pool,
                        )
                    {
                        onchain_liquidity
                            .entry(boundary_pool.tokens)
                            .or_default()
                            .push(OnchainLiquidity {
                                id: liquidity.id.clone(),
                                token_pair: boundary_pool.tokens,
                                source: LiquiditySource::ConstantProduct(boundary_pool),
                            });
                    }
                }
                liquidity::State::WeightedProduct(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::weighted_product::to_boundary_pool(
                            liquidity.address,
                            pool,
                        )
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::WeightedProduct(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::Stable(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::stable::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::Stable(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::LimitOrder(limit_order) => {
                    if let Some(token_pair) = TokenPair::new(
                        limit_order.maker.token.0.into_alloy(),
                        limit_order.taker.token.0.into_alloy(),
                    ) {
                        onchain_liquidity
                            .entry(token_pair)
                            .or_default()
                            .push(OnchainLiquidity {
                                id: liquidity.id.clone(),
                                token_pair,
                                source: LiquiditySource::LimitOrder(limit_order.clone()),
                            })
                    }
                }
                liquidity::State::Concentrated(pool) => {
                    let Some(ref uni_v3_quoter_v2_arc) = uni_v3_quoter_v2 else {
                        // liquidity sources that rely on concentrated pools are disabled
                        return onchain_liquidity;
                    };
                    let fee = pool.fee.0.try_into().expect("fee < (2^24)");

                    let token_pair = to_boundary_token_pair(&pool.tokens);
                    onchain_liquidity
                        .entry(token_pair)
                        .or_default()
                        .push(OnchainLiquidity {
                            id: liquidity.id.clone(),
                            token_pair,
                            source: LiquiditySource::Concentrated(
                                boundary::liquidity::concentrated::Pool {
                                    uni_v3_quoter_contract: uni_v3_quoter_v2_arc.clone(),
                                    address: liquidity.address,
                                    tokens: token_pair,
                                    fee,
                                },
                            ),
                        })
                }
                liquidity::State::GyroE(pool) => {
                    let pool = pool.as_ref();
                    if let Some(boundary_pool) =
                        boundary::liquidity::gyro_e::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::GyroE(Box::new(boundary_pool.clone())),
                                },
                            );
                        }
                    }
                }
                liquidity::State::Gyro2CLP(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::gyro_2clp::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::Gyro2CLP(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::Gyro3CLP(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::gyro_3clp::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::Gyro3CLP(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::BalancerV3ReClamm(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::reclamm::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::ReClamm(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::QuantAmm(pool) => {
                    if let Some(boundary_pool) =
                        boundary::liquidity::quantamm::to_boundary_pool(liquidity.address, pool)
                    {
                        for pair in pool.reserves.token_pairs() {
                            let token_pair = to_boundary_token_pair(&pair);
                            onchain_liquidity.entry(token_pair).or_default().push(
                                OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair,
                                    source: LiquiditySource::QuantAmm(boundary_pool.clone()),
                                },
                            );
                        }
                    }
                }
                liquidity::State::Erc4626(edge) => {
                    if let Some(web3) = erc4626_web3 {
                        let edge_boundary =
                            boundary_erc4626::Edge::new(web3, edge.vault.0, edge.asset.0);
                        if let Some(pair_fw) =
                            TokenPair::new(edge.asset.0.into_alloy(), edge.vault.0.into_alloy())
                        {
                            onchain_liquidity
                                .entry(pair_fw)
                                .or_default()
                                .push(OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair: pair_fw,
                                    source: LiquiditySource::Erc4626(edge_boundary.clone()),
                                });
                        }
                        if let Some(pair_bw) =
                            TokenPair::new(edge.vault.0.into_alloy(), edge.asset.0.into_alloy())
                        {
                            onchain_liquidity
                                .entry(pair_bw)
                                .or_default()
                                .push(OnchainLiquidity {
                                    id: liquidity.id.clone(),
                                    token_pair: pair_bw,
                                    source: LiquiditySource::Erc4626(edge_boundary),
                                });
                        }
                    } else {
                        tracing::debug!(
                            vault = ?edge.vault.0,
                            asset = ?edge.asset.0,
                            "Skipping ERC4626 in baseline routing: no Web3 configured"
                        );
                    }
                }
            };
            onchain_liquidity
        })
}

#[derive(Debug)]
struct OnchainLiquidity {
    id: liquidity::Id,
    token_pair: TokenPair,
    source: LiquiditySource,
}

impl OnchainLiquidity {
    /// Get the pool kind as a string
    fn kind_str(&self) -> &str {
        match &self.source {
            LiquiditySource::ConstantProduct(_) => "constantProduct",
            LiquiditySource::WeightedProduct(_) => "weightedProduct",
            LiquiditySource::Stable(_) => "stable",
            LiquiditySource::GyroE(_) => "gyroE",
            LiquiditySource::Gyro2CLP(_) => "gyro2CLP",
            LiquiditySource::Gyro3CLP(_) => "gyro3CLP",
            LiquiditySource::ReClamm(_) => "reClamm",
            LiquiditySource::QuantAmm(_) => "quantAmm",
            LiquiditySource::LimitOrder(_) => "limitOrder",
            LiquiditySource::Concentrated(_) => "concentrated",
            LiquiditySource::Erc4626(_) => "erc4626",
        }
    }

    /// Get the pool address
    fn address(&self) -> H160 {
        match &self.source {
            LiquiditySource::ConstantProduct(pool) => pool.address,
            LiquiditySource::WeightedProduct(pool) => pool.common.address,
            LiquiditySource::Stable(pool) => pool.common.address,
            LiquiditySource::GyroE(pool) => pool.common.address,
            LiquiditySource::Gyro2CLP(pool) => pool.common.address,
            LiquiditySource::Gyro3CLP(pool) => pool.common.address,
            LiquiditySource::ReClamm(pool) => pool.common.address,
            LiquiditySource::QuantAmm(pool) => pool.common.address,
            LiquiditySource::LimitOrder(_) => H160::zero(),
            LiquiditySource::Concentrated(pool) => pool.address,
            LiquiditySource::Erc4626(_) => H160::zero(),
        }
    }
}

#[derive(Debug)]
enum LiquiditySource {
    ConstantProduct(boundary::liquidity::constant_product::Pool),
    WeightedProduct(boundary::liquidity::weighted_product::Pool),
    Stable(boundary::liquidity::stable::Pool),
    GyroE(Box<boundary::liquidity::gyro_e::Pool>),
    Gyro2CLP(boundary::liquidity::gyro_2clp::Pool),
    Gyro3CLP(boundary::liquidity::gyro_3clp::Pool),
    ReClamm(boundary::liquidity::reclamm::Pool),
    LimitOrder(liquidity::limit_order::LimitOrder),
    Concentrated(boundary::liquidity::concentrated::Pool),
    QuantAmm(boundary::liquidity::quantamm::Pool),
    Erc4626(boundary_erc4626::Edge),
}

impl BaselineSolvable for OnchainLiquidity {
    async fn get_amount_out(&self, out_token: H160, input: (U256, H160)) -> Option<U256> {
        match &self.source {
            LiquiditySource::ConstantProduct(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::WeightedProduct(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::Stable(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::GyroE(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::Gyro2CLP(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::Gyro3CLP(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::ReClamm(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::QuantAmm(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::LimitOrder(limit_order) => {
                limit_order.get_amount_out(out_token, input).await
            }
            LiquiditySource::Concentrated(pool) => pool.get_amount_out(out_token, input).await,
            LiquiditySource::Erc4626(edge) => edge.get_amount_out(out_token, input).await,
        }
    }

    async fn get_amount_in(&self, in_token: H160, out: (U256, H160)) -> Option<U256> {
        match &self.source {
            LiquiditySource::ConstantProduct(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::WeightedProduct(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::Stable(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::GyroE(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::Gyro2CLP(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::Gyro3CLP(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::ReClamm(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::QuantAmm(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::LimitOrder(limit_order) => {
                limit_order.get_amount_in(in_token, out).await
            }
            LiquiditySource::Concentrated(pool) => pool.get_amount_in(in_token, out).await,
            LiquiditySource::Erc4626(edge) => edge.get_amount_in(in_token, out).await,
        }
    }

    async fn gas_cost(&self) -> usize {
        match &self.source {
            LiquiditySource::ConstantProduct(pool) => pool.gas_cost().await,
            LiquiditySource::WeightedProduct(pool) => pool.gas_cost().await,
            LiquiditySource::Stable(pool) => pool.gas_cost().await,
            LiquiditySource::GyroE(pool) => pool.gas_cost().await,
            LiquiditySource::Gyro2CLP(pool) => pool.gas_cost().await,
            LiquiditySource::Gyro3CLP(pool) => pool.gas_cost().await,
            LiquiditySource::ReClamm(pool) => pool.gas_cost().await,
            LiquiditySource::QuantAmm(pool) => pool.gas_cost().await,
            LiquiditySource::LimitOrder(limit_order) => limit_order.gas_cost().await,
            LiquiditySource::Concentrated(pool) => pool.gas_cost().await,
            LiquiditySource::Erc4626(edge) => edge.gas_cost().await,
        }
    }
}

fn to_boundary_base_tokens(
    weth: &eth::WethAddress,
    base_tokens: &HashSet<eth::TokenAddress>,
) -> BaseTokens {
    let base_tokens = base_tokens.iter().map(|token| token.0).collect::<Vec<_>>();
    BaseTokens::new(weth.0, &base_tokens)
}

fn to_boundary_token_pair(pair: &liquidity::TokenPair) -> TokenPair {
    let (a, b) = pair.get();
    TokenPair::new(a.0.into_alloy(), b.0.into_alloy()).unwrap()
}

/// Extract pool parameters for logging purposes
fn extract_pool_params(liquidity: &liquidity::Liquidity) -> serde_json::Value {
    use serde_json::json;

    match &liquidity.state {
        liquidity::State::ConstantProduct(_pool) => {
            json!({
                "kind": "constantProduct",
                // Simplified for constant product pools
            })
        }
        liquidity::State::WeightedProduct(pool) => {
            json!({
                "kind": "weightedProduct",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                    "weight": format!("{}/{}", r.weight.numer(), r.weight.denom()),
                    "scalingFactor": format!("{}/{}", r.scale.get().numer(), r.scale.get().denom()),
                    "rate": format!("{}/{}", r.rate.numer(), r.rate.denom()),
                })).collect::<Vec<_>>(),
            })
        }
        liquidity::State::Stable(pool) => {
            json!({
                "kind": "stable",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "amplificationParameter": format!("{}/{}", pool.amplification_parameter.numer(), pool.amplification_parameter.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                    "scalingFactor": format!("{}/{}", r.scale.get().numer(), r.scale.get().denom()),
                    "rate": format!("{}/{}", r.rate.numer(), r.rate.denom()),
                })).collect::<Vec<_>>(),
            })
        }
        liquidity::State::GyroE(pool) => {
            json!({
                "kind": "gyroE",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                    "scalingFactor": format!("{}/{}", r.scale.get().numer(), r.scale.get().denom()),
                    "rate": format!("{}/{}", r.rate.numer(), r.rate.denom()),
                })).collect::<Vec<_>>(),
                "params": json!({
                    "alpha": format!("{}/{}", pool.params_alpha.numer(), pool.params_alpha.denom()),
                    "beta": format!("{}/{}", pool.params_beta.numer(), pool.params_beta.denom()),
                    "c": format!("{}/{}", pool.params_c.numer(), pool.params_c.denom()),
                    "s": format!("{}/{}", pool.params_s.numer(), pool.params_s.denom()),
                    "lambda": format!("{}/{}", pool.params_lambda.numer(), pool.params_lambda.denom()),
                }),
            })
        }
        liquidity::State::Gyro2CLP(pool) => {
            json!({
                "kind": "gyro2CLP",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                })).collect::<Vec<_>>(),
                "sqrtAlpha": format!("{}/{}", pool.sqrt_alpha.numer(), pool.sqrt_alpha.denom()),
                "sqrtBeta": format!("{}/{}", pool.sqrt_beta.numer(), pool.sqrt_beta.denom()),
            })
        }
        liquidity::State::Gyro3CLP(pool) => {
            json!({
                "kind": "gyro3CLP",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                })).collect::<Vec<_>>(),
                "root3Alpha": format!("{}/{}", pool.root3_alpha.numer(), pool.root3_alpha.denom()),
            })
        }
        liquidity::State::BalancerV3ReClamm(pool) => {
            json!({
                "kind": "reClamm",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                })).collect::<Vec<_>>(),
            })
        }
        liquidity::State::QuantAmm(pool) => {
            json!({
                "kind": "quantAmm",
                "fee": format!("{}/{}", pool.fee.numer(), pool.fee.denom()),
                "reserves": pool.reserves.iter().map(|r| json!({
                    "token": format!("{:#x}", r.asset.token.0),
                    "balance": r.asset.amount.to_string(),
                })).collect::<Vec<_>>(),
            })
        }
        liquidity::State::Concentrated(_pool) => {
            json!({
                "kind": "concentrated",
                // Concentrated liquidity pools are complex, just note the type
            })
        }
        liquidity::State::Erc4626(_edge) => {
            json!({
                "kind": "erc4626",
                // ERC4626 is a wrapper, minimal info needed
            })
        }
        liquidity::State::LimitOrder(order) => {
            json!({
                "kind": "limitOrder",
                "maker": format!("{:#x}", order.maker.token.0),
                "taker": format!("{:#x}", order.taker.token.0),
                "makerAmount": order.maker.amount.to_string(),
                "takerAmount": order.taker.amount.to_string(),
            })
        }
    }
}
