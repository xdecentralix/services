use {
    crate::{
        api::routes::Error,
        domain::{auction, eth, liquidity, order},
        infra::liquidity_client::{LiquidityClient, LiquidityRequest},
        util::conv,
    },
    bigdecimal::{FromPrimitive, ToPrimitive},
    itertools::Itertools,
    solvers_dto::auction::*,
    std::collections::HashSet,
};

/// Extract token pairs from auction orders for liquidity fetching
/// This creates comprehensive routing pairs including base tokens
fn extract_token_pairs_from_auction(
    auction: &Auction,
    base_tokens: Option<&[eth::H160]>,
) -> Vec<(eth::H160, eth::H160)> {
    let mut result = HashSet::new();

    // Extract direct pairs from orders
    for order in &auction.orders {
        if order.sell_token != order.buy_token {
            let pair = if order.sell_token < order.buy_token {
                (order.sell_token, order.buy_token)
            } else {
                (order.buy_token, order.sell_token)
            };
            result.insert(pair);
        }
    }

    // Expand with base token pairs if base tokens are provided
    if let Some(base_tokens) = base_tokens {
        let base_token_set: HashSet<_> = base_tokens.iter().copied().collect();

        // For each pair, add connections to base tokens
        let original_pairs: Vec<_> = result.iter().copied().collect();
        for (token_a, token_b) in original_pairs {
            // Add pairs between each token and all base tokens
            for &base_token in &base_token_set {
                if base_token != token_a {
                    let pair = if base_token < token_a {
                        (base_token, token_a)
                    } else {
                        (token_a, base_token)
                    };
                    result.insert(pair);
                }
                if base_token != token_b {
                    let pair = if base_token < token_b {
                        (base_token, token_b)
                    } else {
                        (token_b, base_token)
                    };
                    result.insert(pair);
                }
            }
        }

        // Add all base token pairs (for routing between base tokens)
        let base_tokens_vec: Vec<_> = base_token_set.iter().copied().collect();
        for (i, &token_a) in base_tokens_vec.iter().enumerate() {
            for &token_b in &base_tokens_vec[i + 1..] {
                let pair = if token_a < token_b {
                    (token_a, token_b)
                } else {
                    (token_b, token_a)
                };
                result.insert(pair);
            }
        }
    }

    result.into_iter().collect()
}

/// Converts a data transfer object into its domain object representation.
/// If liquidity_client is provided and auction has empty liquidity, fetches
/// independently.
pub async fn into_domain(
    auction: Auction,
    liquidity_client: Option<&LiquidityClient>,
    base_tokens: Option<&[eth::H160]>,
    protocols: Option<&[String]>,
) -> Result<auction::Auction, Error> {
    Ok(auction::Auction {
        id: match auction.id {
            Some(id) => auction::Id::Solve(id),
            None => auction::Id::Quote,
        },
        tokens: auction::Tokens(
            auction
                .tokens
                .iter()
                .map(|(address, token)| {
                    (
                        eth::TokenAddress(*address),
                        auction::Token {
                            decimals: token.decimals,
                            symbol: token.symbol.clone(),
                            reference_price: token
                                .reference_price
                                .map(eth::Ether)
                                .map(auction::Price),
                            available_balance: token.available_balance,
                            trusted: token.trusted,
                        },
                    )
                })
                .collect(),
        ),
        orders: auction
            .orders
            .iter()
            .map(|order| order::Order {
                uid: order::Uid(order.uid),
                sell: eth::Asset {
                    token: eth::TokenAddress(order.sell_token),
                    amount: order.sell_amount,
                },
                buy: eth::Asset {
                    token: eth::TokenAddress(order.buy_token),
                    amount: order.buy_amount,
                },
                side: match order.kind {
                    Kind::Buy => order::Side::Buy,
                    Kind::Sell => order::Side::Sell,
                },
                class: match order.class {
                    Class::Market => order::Class::Market,
                    Class::Limit => order::Class::Limit,
                },
                partially_fillable: order.partially_fillable,
                flashloan_hint: order
                    .flashloan_hint
                    .clone()
                    .map(|hint| order::FlashloanHint {
                        lender: eth::Address(hint.lender),
                        borrower: eth::Address(hint.borrower),
                        token: eth::TokenAddress(hint.token),
                        amount: hint.amount,
                    }),
            })
            .collect(),
        liquidity: {
            if auction.liquidity.is_empty() && liquidity_client.is_some() {
                // Fetch liquidity independently from the liquidity-driver API
                let client = liquidity_client.unwrap();
                let token_pairs = extract_token_pairs_from_auction(&auction, base_tokens);

                tracing::info!(
                    auction_id = auction.id,
                    pairs_count = token_pairs.len(),
                    "Auction has empty liquidity - fetching from liquidity-driver API"
                );

                // Use the auction deadline to estimate a reasonable block number
                // This is approximate but better than 0
                let estimated_block_number = match auction.deadline.timestamp() {
                    ts if ts > 0 => {
                        // Rough estimate: ~12 seconds per block on Ethereum
                        let current_time = chrono::Utc::now().timestamp();
                        let blocks_in_future = (ts - current_time).max(0) / 12;
                        // Add current estimated block (rough estimate)
                        18_000_000u64 + blocks_in_future as u64
                    }
                    _ => 18_000_000u64, // Fallback to reasonable mainnet block number
                };

                let request = LiquidityRequest {
                    auction_id: auction.id.unwrap_or(0) as u64,
                    tokens: auction.tokens.keys().copied().collect(),
                    token_pairs,
                    block_number: estimated_block_number,
                    protocols: protocols.map(|p| p.to_vec()).unwrap_or_else(|| {
                        vec!["balancer_v2".to_string(), "uniswap_v2".to_string()]
                    }),
                };

                match client.fetch_liquidity(request).await {
                    Ok(response) => {
                        tracing::info!(
                            auction_id = auction.id,
                            liquidity_count = response.liquidity.len(),
                            "Successfully fetched liquidity from API"
                        );

                        // Process the fetched liquidity
                        response
                            .liquidity
                            .iter()
                            .map(|liquidity| convert_dto_liquidity_to_domain(liquidity))
                            .try_collect()?
                    }
                    Err(e) => {
                        tracing::warn!(
                            auction_id = auction.id,
                            error = ?e,
                            "Failed to fetch liquidity from API - continuing with empty liquidity"
                        );
                        Vec::new() // Graceful degradation
                    }
                }
            } else {
                // Use existing embedded liquidity
                auction
                    .liquidity
                    .iter()
                    .map(|liquidity| convert_dto_liquidity_to_domain(liquidity))
                    .try_collect()?
            }
        },
        gas_price: auction::GasPrice(eth::Ether(auction.effective_gas_price)),
        deadline: auction::Deadline(auction.deadline),
    })
}

/// Helper function to convert DTO liquidity to domain liquidity
fn convert_dto_liquidity_to_domain(liquidity: &Liquidity) -> Result<liquidity::Liquidity, Error> {
    match liquidity {
        Liquidity::ConstantProduct(liquidity) => constant_product_pool::to_domain(liquidity),
        Liquidity::WeightedProduct(liquidity) => weighted_product_pool::to_domain(liquidity),
        Liquidity::Stable(liquidity) => stable_pool::to_domain(liquidity),
        Liquidity::ConcentratedLiquidity(liquidity) => {
            concentrated_liquidity_pool::to_domain(liquidity)
        }
        Liquidity::GyroE(liquidity) => gyro_e_pool::to_domain(liquidity),
        Liquidity::Gyro2CLP(liquidity) => gyro_2clp_pool::to_domain(liquidity),
        Liquidity::Gyro3CLP(liquidity) => gyro_3clp_pool::to_domain(liquidity),
        Liquidity::LimitOrder(liquidity) => Ok(foreign_limit_order::to_domain(liquidity)),
        Liquidity::Erc4626(liquidity) => erc4626::to_domain(liquidity),
        Liquidity::ReClamm(liquidity) => reclamm_pool::to_domain(liquidity),
        Liquidity::QuantAmm(liquidity) => quant_amm_pool::to_domain(liquidity),
        Liquidity::StableSurge(liquidity) => stable_surge_pool::to_domain(liquidity),
    }
}

mod erc4626 {
    use super::*;
    pub fn to_domain(edge: &Erc4626Edge) -> Result<liquidity::Liquidity, Error> {
        Ok(liquidity::Liquidity {
            id: liquidity::Id(edge.id.clone()),
            address: edge.vault,
            gas: eth::Gas(edge.gas_estimate),
            state: liquidity::State::Erc4626(liquidity::erc4626::Edge {
                asset: eth::TokenAddress(edge.asset),
                vault: eth::TokenAddress(edge.vault),
            }),
        })
    }
}

mod constant_product_pool {
    use {super::*, itertools::Itertools};

    pub fn to_domain(pool: &ConstantProductPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let (a, b) = pool
                .tokens
                .iter()
                .map(|(token, reserve)| eth::Asset {
                    token: eth::TokenAddress(*token),
                    amount: reserve.balance,
                })
                .collect_tuple()
                .ok_or("invalid number of constant product tokens")?;
            liquidity::constant_product::Reserves::new(a, b)
                .ok_or("invalid constant product pool reserves")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::ConstantProduct(liquidity::constant_product::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid constant product fee")?,
            }),
        })
    }
}

mod weighted_product_pool {
    use super::*;
    pub fn to_domain(pool: &WeightedProductPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::weighted_product::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        weight: conv::decimal_to_rational(&token.weight)
                            .ok_or("invalid token weight")?,
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::weighted_product::Reserves::new(entries)
                .ok_or("duplicate weighted token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::WeightedProduct(liquidity::weighted_product::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid weighted product fee")?,
                version: match pool.version {
                    WeightedProductVersion::V0 => liquidity::weighted_product::Version::V0,
                    WeightedProductVersion::V3Plus => liquidity::weighted_product::Version::V3Plus,
                },
            }),
        })
    }
}

mod stable_pool {
    use super::*;
    pub fn to_domain(pool: &StablePool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::stable::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::stable::Reserves::new(entries).ok_or("duplicate stable token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::Stable(liquidity::stable::Pool {
                reserves,
                amplification_parameter: conv::decimal_to_rational(&pool.amplification_parameter)
                    .ok_or("invalid amplification parameter")?,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid stable pool fee")?,
            }),
        })
    }
}

mod concentrated_liquidity_pool {
    use {super::*, bigdecimal::BigDecimal, itertools::Itertools};

    pub fn to_domain(pool: &ConcentratedLiquidityPool) -> Result<liquidity::Liquidity, Error> {
        let tokens = {
            let (a, b) = pool
                .tokens
                .iter()
                .copied()
                .map(eth::TokenAddress)
                .collect_tuple()
                .ok_or("invalid number of concentrated liquidity pool tokens")?;
            liquidity::TokenPair::new(a, b)
                .ok_or("duplicate concentrated liquidity pool token address")?
        };
        // Convert fee from decimal to the format expected by the UniswapV3 smart
        // contract. Uniswap expresses fees in hundredths of a basis point
        // (1e-6):
        //   - 0.003 (0.3%) → 0.003 × 1,000,000 = 3000 (i.e., 3000 × 1e-6 = 0.003)
        //   - 1 bps = 0.0001 → 1 bps = 100 units in Uniswap format
        // So multiplying by 1,000,000 converts a decimal fee into Uniswap fee units.
        let bps = BigDecimal::from_f32(1_000_000.).unwrap();

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::Concentrated(liquidity::concentrated::Pool {
                tokens,
                fee: liquidity::concentrated::Fee(
                    (pool.fee.clone() * bps)
                        .to_u32()
                        .ok_or("invalid concentrated liquidity pool fee")?,
                ),
            }),
        })
    }
}

mod foreign_limit_order {
    use super::*;

    pub fn to_domain(order: &ForeignLimitOrder) -> liquidity::Liquidity {
        liquidity::Liquidity {
            id: liquidity::Id(order.id.clone()),
            address: order.address,
            gas: eth::Gas(order.gas_estimate),
            state: liquidity::State::LimitOrder(liquidity::limit_order::LimitOrder {
                maker: eth::Asset {
                    token: eth::TokenAddress(order.maker_token),
                    amount: order.maker_amount,
                },
                taker: eth::Asset {
                    token: eth::TokenAddress(order.taker_token),
                    amount: order.taker_amount,
                },
                fee: liquidity::limit_order::TakerAmount(order.taker_token_fee_amount),
            }),
        }
    }
}

mod gyro_e_pool {
    use super::*;

    pub fn to_domain(pool: &GyroEPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::gyro_e::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::gyro_e::Reserves::new(entries).ok_or("duplicate GyroE token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::GyroE(Box::new(liquidity::gyro_e::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid GyroE pool fee")?,
                version: match pool.version {
                    GyroEVersion::V1 => liquidity::gyro_e::Version::V1,
                },
                // Convert all Gyro E-CLP static parameters from BigDecimal to SignedRational
                // These parameters can be negative, so we use the new signed conversion function
                params_alpha: conv::decimal_to_signed_rational(&pool.params_alpha)
                    .ok_or("invalid params_alpha")?,
                params_beta: conv::decimal_to_signed_rational(&pool.params_beta)
                    .ok_or("invalid params_beta")?,
                params_c: conv::decimal_to_signed_rational(&pool.params_c)
                    .ok_or("invalid params_c")?,
                params_s: conv::decimal_to_signed_rational(&pool.params_s)
                    .ok_or("invalid params_s")?,
                params_lambda: conv::decimal_to_signed_rational(&pool.params_lambda)
                    .ok_or("invalid params_lambda")?,
                tau_alpha_x: conv::decimal_to_signed_rational(&pool.tau_alpha_x)
                    .ok_or("invalid tau_alpha_x")?,
                tau_alpha_y: conv::decimal_to_signed_rational(&pool.tau_alpha_y)
                    .ok_or("invalid tau_alpha_y")?,
                tau_beta_x: conv::decimal_to_signed_rational(&pool.tau_beta_x)
                    .ok_or("invalid tau_beta_x")?,
                tau_beta_y: conv::decimal_to_signed_rational(&pool.tau_beta_y)
                    .ok_or("invalid tau_beta_y")?,
                u: conv::decimal_to_signed_rational(&pool.u).ok_or("invalid u")?,
                v: conv::decimal_to_signed_rational(&pool.v).ok_or("invalid v")?,
                w: conv::decimal_to_signed_rational(&pool.w).ok_or("invalid w")?,
                z: conv::decimal_to_signed_rational(&pool.z).ok_or("invalid z")?,
                d_sq: conv::decimal_to_signed_rational(&pool.d_sq).ok_or("invalid d_sq")?,
            })),
        })
    }
}

mod gyro_2clp_pool {
    use super::*;

    pub fn to_domain(pool: &Gyro2CLPPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::gyro_2clp::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::gyro_2clp::Reserves::new(entries)
                .ok_or("duplicate Gyro2CLP token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::Gyro2CLP(liquidity::gyro_2clp::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid Gyro2CLP pool fee")?,
                version: match pool.version {
                    Gyro2CLPVersion::V1 => liquidity::gyro_2clp::Version::V1,
                },
                // Convert Gyro 2-CLP static parameters from BigDecimal to SignedRational
                sqrt_alpha: conv::decimal_to_signed_rational(&pool.sqrt_alpha)
                    .ok_or("invalid sqrt_alpha")?,
                sqrt_beta: conv::decimal_to_signed_rational(&pool.sqrt_beta)
                    .ok_or("invalid sqrt_beta")?,
            }),
        })
    }
}

mod gyro_3clp_pool {
    use super::*;

    pub fn to_domain(pool: &Gyro3CLPPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = pool
            .tokens
            .iter()
            .map(|(address, token)| {
                Ok(liquidity::gyro_3clp::Reserve {
                    asset: eth::Asset {
                        token: (*address).into(),
                        amount: token.balance,
                    },
                    scale: liquidity::ScalingFactor::new(
                        conv::decimal_to_rational(&token.scaling_factor)
                            .ok_or("invalid scaling factor")?,
                    )
                    .ok_or("invalid scaling factor")?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::Gyro3CLP(liquidity::gyro_3clp::Pool {
                reserves: liquidity::gyro_3clp::Reserves::new(reserves)
                    .ok_or("invalid 3-CLP reserves")?,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid Gyro3CLP pool fee")?,
                version: match pool.version {
                    Gyro3CLPVersion::V1 => liquidity::gyro_3clp::Version::V1,
                },
                root3_alpha: conv::decimal_to_rational(&pool.root3_alpha)
                    .ok_or("invalid root3_alpha")?,
            }),
        })
    }
}

mod reclamm_pool {
    use super::*;
    pub fn to_domain(pool: &ReClammPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::reclamm::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::reclamm::Reserves::try_new(entries)
                .map_err(|_| "duplicate token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::BalancerV3ReClamm(liquidity::reclamm::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid fee")?,
                last_virtual_balances: pool
                    .last_virtual_balances
                    .iter()
                    .map(|v| conv::decimal_to_rational(v).ok_or("invalid last_virtual_balance"))
                    .collect::<Result<Vec<_>, _>>()?,
                daily_price_shift_base: conv::decimal_to_rational(&pool.daily_price_shift_base)
                    .ok_or("invalid daily_price_shift_base")?,
                last_timestamp: pool.last_timestamp,
                centeredness_margin: conv::decimal_to_rational(&pool.centeredness_margin)
                    .ok_or("invalid centeredness_margin")?,
                start_fourth_root_price_ratio: conv::decimal_to_rational(
                    &pool.start_fourth_root_price_ratio,
                )
                .ok_or("invalid start_fourth_root_price_ratio")?,
                end_fourth_root_price_ratio: conv::decimal_to_rational(
                    &pool.end_fourth_root_price_ratio,
                )
                .ok_or("invalid end_fourth_root_price_ratio")?,
                price_ratio_update_start_time: pool.price_ratio_update_start_time,
                price_ratio_update_end_time: pool.price_ratio_update_end_time,
            }),
        })
    }
}

mod stable_surge_pool {
    use super::*;
    pub fn to_domain(pool: &StableSurgePool) -> Result<liquidity::Liquidity, Error> {
        // External solvers receive StableSurge pool data but convert them to regular
        // stable pools for their own pathfinding. They use the current
        // effective fee from the DTO, not the dynamic surge calculations (which
        // happen in the internal solver/driver).
        let reserves = {
            let mut entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::stable::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;

            // Sort by token address (stable pools require sorted tokens)
            entries.sort_unstable_by_key(|reserve| reserve.asset.token);

            liquidity::stable::Reserves::new(entries)
                .ok_or("duplicate stable surge token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::Stable(liquidity::stable::Pool {
                reserves,
                amplification_parameter: conv::decimal_to_rational(&pool.amplification_parameter)
                    .ok_or("invalid amplification parameter")?,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid stable surge pool fee")?,
            }),
        })
    }
}

mod quant_amm_pool {
    use super::*;
    pub fn to_domain(pool: &QuantAmmPool) -> Result<liquidity::Liquidity, Error> {
        let reserves = {
            let entries = pool
                .tokens
                .iter()
                .map(|(address, token)| {
                    Ok(liquidity::quantamm::Reserve {
                        asset: eth::Asset {
                            token: eth::TokenAddress(*address),
                            amount: token.balance,
                        },
                        scale: conv::decimal_to_rational(&token.scaling_factor)
                            .and_then(liquidity::ScalingFactor::new)
                            .ok_or("invalid token scaling factor")?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            liquidity::quantamm::Reserves::new(entries)
                .ok_or("duplicate QuantAMM token addresses")?
        };

        Ok(liquidity::Liquidity {
            id: liquidity::Id(pool.id.clone()),
            address: pool.address,
            gas: eth::Gas(pool.gas_estimate),
            state: liquidity::State::QuantAmm(liquidity::quantamm::Pool {
                reserves,
                fee: conv::decimal_to_rational(&pool.fee).ok_or("invalid fee")?,
                version: match pool.version {
                    QuantAmmVersion::V1 => liquidity::quantamm::Version::V1,
                },
                max_trade_size_ratio: conv::decimal_to_rational(&pool.max_trade_size_ratio)
                    .ok_or("invalid max_trade_size_ratio")?,
                first_four_weights_and_multipliers: pool
                    .first_four_weights_and_multipliers
                    .iter()
                    .map(|v| {
                        conv::decimal_to_signed_rational(v).ok_or("invalid weights_and_multiplier")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                second_four_weights_and_multipliers: pool
                    .second_four_weights_and_multipliers
                    .iter()
                    .map(|v| {
                        conv::decimal_to_signed_rational(v).ok_or("invalid weights_and_multiplier")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                last_update_time: pool.last_update_time,
                last_interop_time: pool.last_interop_time,
                current_timestamp: pool.current_timestamp,
            }),
        })
    }
}
