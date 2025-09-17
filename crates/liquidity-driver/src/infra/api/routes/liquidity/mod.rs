use {
    crate::{
        domain::{eth, liquidity},
        infra::{
            api::{State, error},
            liquidity::fetcher::AtBlock,
            observe,
        },
        util::conv::{rational_to_big_decimal, u256::U256Ext},
    },
    std::collections::HashSet,
    tracing::Instrument,
};

mod dto;

pub use dto::*;

/// Register the liquidity route with the router
pub(in crate::infra::api) fn liquidity(router: axum::Router<State>) -> axum::Router<State> {
    router.route("/api/v1/liquidity", axum::routing::post(route))
}

/// Main handler for the /api/v1/liquidity endpoint
async fn route(
    state: axum::extract::State<State>,
    req: axum::Json<LiquidityRequest>,
) -> Result<axum::Json<ApiLiquidityResponse>, (hyper::StatusCode, axum::Json<error::Error>)> {
    let auction_id = req.auction_id; // Extract before moving req

    let handle_request = async {
        let request = req.0;

        // Convert token pairs to the domain format
        let pairs = request
            .token_pairs
            .into_iter()
            .map(|(a, b)| liquidity::TokenPair::try_new(a.into(), b.into()))
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|_| LiquidityError::InvalidTokenPair)?;

        observe::fetching_liquidity();

        // Fetch liquidity using the existing liquidity fetcher
        let domain_liquidity = state.liquidity().fetch(&pairs, AtBlock::Latest).await;

        observe::fetched_liquidity(&domain_liquidity);

        // Convert domain liquidity to solvers-dto format
        let liquidity_dto = domain_liquidity
            .into_iter()
            .filter_map(|liq| match convert_domain_to_dto(liq) {
                Ok(dto) => Some(dto),
                Err(e) => {
                    tracing::warn!(
                        liquidity_id = ?e,
                        "Failed to convert domain liquidity to DTO, skipping"
                    );
                    None
                }
            })
            .collect();

        let response = LiquidityResponse {
            auction_id: request.auction_id,
            liquidity: liquidity_dto,
            block_number: request.block_number,
            timestamp: chrono::Utc::now().timestamp() as u64,
        };

        Ok(axum::Json(ApiLiquidityResponse { result: response }))
    };

    handle_request
        .instrument(tracing::info_span!(
            "/api/v1/liquidity",
            auction_id = auction_id
        ))
        .await
}

/// Convert domain liquidity types to solvers_dto types
fn convert_domain_to_dto(
    liquidity: liquidity::Liquidity,
) -> Result<solvers_dto::auction::Liquidity, LiquidityError> {
    match liquidity.kind {
        liquidity::Kind::UniswapV2(pool) => Ok(solvers_dto::auction::Liquidity::ConstantProduct(
            solvers_dto::auction::ConstantProductPool {
                id: liquidity.id.0.to_string(),
                address: pool.address.0.into(),
                router: pool.router.0.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|asset| {
                        (
                            asset.token.into(),
                            solvers_dto::auction::ConstantProductReserve {
                                balance: asset.amount.into(),
                            },
                        )
                    })
                    .collect(),
                fee: bigdecimal::BigDecimal::new(3.into(), 3),
            },
        )),
        liquidity::Kind::UniswapV3(pool) => {
            Ok(solvers_dto::auction::Liquidity::ConcentratedLiquidity(
                solvers_dto::auction::ConcentratedLiquidityPool {
                    id: liquidity.id.0.to_string(),
                    address: pool.address.0,
                    router: pool.router.into(),
                    gas_estimate: liquidity.gas.0,
                    tokens: vec![pool.tokens.get().0.into(), pool.tokens.get().1.into()],
                    sqrt_price: pool.sqrt_price.0,
                    liquidity: pool.liquidity.0,
                    tick: pool.tick.0,
                    liquidity_net: pool
                        .liquidity_net
                        .iter()
                        .map(|(key, value)| (key.0, value.0))
                        .collect(),
                    fee: rational_to_big_decimal(&pool.fee.0),
                },
            ))
        }

        liquidity::Kind::BalancerV2Weighted(pool) => {
            Ok(solvers_dto::auction::Liquidity::WeightedProduct(
                solvers_dto::auction::WeightedProductPool {
                    id: liquidity.id.0.to_string(),
                    address: pool.id.address().into(),
                    balancer_pool_id: pool.id.into(),
                    gas_estimate: liquidity.gas.0.into(),
                    tokens: pool
                        .reserves
                        .iter()
                        .map(|r| {
                            (
                                r.asset.token.into(),
                                solvers_dto::auction::WeightedProductReserve {
                                    balance: r.asset.amount.into(),
                                    scaling_factor: scaling_factor_to_decimal(r.scale),
                                    weight: weight_to_decimal(r.weight),
                                },
                            )
                        })
                        .collect(),
                    fee: fee_to_decimal(pool.fee),
                    version: match pool.version {
                        liquidity::balancer::v2::weighted::Version::V0 => {
                            solvers_dto::auction::WeightedProductVersion::V0
                        }
                        liquidity::balancer::v2::weighted::Version::V3Plus => {
                            solvers_dto::auction::WeightedProductVersion::V3Plus
                        }
                    },
                },
            ))
        }

        liquidity::Kind::BalancerV3Weighted(pool) => {
            Ok(solvers_dto::auction::Liquidity::WeightedProduct(
                solvers_dto::auction::WeightedProductPool {
                    id: liquidity.id.0.to_string(),
                    address: pool.id.address().into(),
                    balancer_pool_id: {
                        let pool_id_h160: eth::H160 = pool.id.into();
                        pool_id_h160.into()
                    },
                    gas_estimate: liquidity.gas.0.into(),
                    tokens: pool
                        .reserves
                        .iter()
                        .map(|r| {
                            (
                                r.asset.token.into(),
                                solvers_dto::auction::WeightedProductReserve {
                                    balance: r.asset.amount.into(),
                                    scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                                    weight: weight_to_decimal_v3(r.weight),
                                },
                            )
                        })
                        .collect(),
                    fee: fee_to_decimal_v3(pool.fee),
                    version: match pool.version {
                        liquidity::balancer::v3::weighted::Version::V1 => {
                            // V3 V1 pools use the same math as V2 V3Plus pools
                            solvers_dto::auction::WeightedProductVersion::V3Plus
                        }
                    },
                },
            ))
        }

        liquidity::Kind::BalancerV2Stable(pool) => Ok(solvers_dto::auction::Liquidity::Stable(
            solvers_dto::auction::StablePool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: pool.id.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::StableReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal(r.scale),
                            },
                        )
                    })
                    .collect(),
                amplification_parameter: rational_to_big_decimal(&num::BigRational::new(
                    pool.amplification_parameter.factor().to_big_int(),
                    pool.amplification_parameter.precision().to_big_int(),
                )),
                fee: fee_to_decimal(pool.fee),
            },
        )),

        liquidity::Kind::BalancerV3Stable(pool) => Ok(solvers_dto::auction::Liquidity::Stable(
            solvers_dto::auction::StablePool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: {
                    let pool_id_h160: eth::H160 = pool.id.into();
                    pool_id_h160.into()
                },
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::StableReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                amplification_parameter: rational_to_big_decimal(&num::BigRational::new(
                    pool.amplification_parameter.factor().to_big_int(),
                    pool.amplification_parameter.precision().to_big_int(),
                )),
                fee: fee_to_decimal_v3(pool.fee),
            },
        )),

        liquidity::Kind::BalancerV3StableSurge(pool) => Ok(
            solvers_dto::auction::Liquidity::StableSurge(solvers_dto::auction::StableSurgePool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: {
                    let pool_id_h160: eth::H160 = pool.id.into();
                    pool_id_h160.into()
                },
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::StableReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                amplification_parameter: rational_to_big_decimal(&num::BigRational::new(
                    pool.amplification_parameter.factor().to_big_int(),
                    pool.amplification_parameter.precision().to_big_int(),
                )),
                fee: fee_to_decimal_v3(pool.fee),
                surge_threshold_percentage: surge_threshold_to_decimal_v3(
                    pool.surge_threshold_percentage.clone(),
                ),
                max_surge_fee_percentage: max_surge_fee_to_decimal_v3(
                    pool.max_surge_fee_percentage.clone(),
                ),
            }),
        ),

        liquidity::Kind::BalancerV2GyroE(pool) => Ok(solvers_dto::auction::Liquidity::GyroE(
            Box::new(solvers_dto::auction::GyroEPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: pool.id.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::GyroEReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v2::gyro_e::Version::V1 => {
                        solvers_dto::auction::GyroEVersion::V1
                    }
                },
                params_alpha: signed_fixed_point_to_decimal(pool.params_alpha),
                params_beta: signed_fixed_point_to_decimal(pool.params_beta),
                params_c: signed_fixed_point_to_decimal(pool.params_c),
                params_s: signed_fixed_point_to_decimal(pool.params_s),
                params_lambda: signed_fixed_point_to_decimal(pool.params_lambda),
                tau_alpha_x: signed_fixed_point_to_decimal(pool.tau_alpha_x),
                tau_alpha_y: signed_fixed_point_to_decimal(pool.tau_alpha_y),
                tau_beta_x: signed_fixed_point_to_decimal(pool.tau_beta_x),
                tau_beta_y: signed_fixed_point_to_decimal(pool.tau_beta_y),
                u: signed_fixed_point_to_decimal(pool.u),
                v: signed_fixed_point_to_decimal(pool.v),
                w: signed_fixed_point_to_decimal(pool.w),
                z: signed_fixed_point_to_decimal(pool.z),
                d_sq: signed_fixed_point_to_decimal(pool.d_sq),
            }),
        )),

        liquidity::Kind::BalancerV2Gyro2CLP(pool) => Ok(solvers_dto::auction::Liquidity::Gyro2CLP(
            solvers_dto::auction::Gyro2CLPPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: pool.id.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::Gyro2CLPReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v2::gyro_2clp::Version::V1 => {
                        solvers_dto::auction::Gyro2CLPVersion::V1
                    }
                },
                sqrt_alpha: signed_fixed_point_to_decimal_gyro_2clp(pool.sqrt_alpha),
                sqrt_beta: signed_fixed_point_to_decimal_gyro_2clp(pool.sqrt_beta),
            },
        )),

        liquidity::Kind::BalancerV2Gyro3CLP(pool) => Ok(solvers_dto::auction::Liquidity::Gyro3CLP(
            solvers_dto::auction::Gyro3CLPPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: pool.id.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::Gyro3CLPReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v2::gyro_3clp::Version::V1 => {
                        solvers_dto::auction::Gyro3CLPVersion::V1
                    }
                },
                root3_alpha: fixed_point_to_decimal(pool.root3_alpha),
            },
        )),

        liquidity::Kind::BalancerV3GyroE(pool) => Ok(solvers_dto::auction::Liquidity::GyroE(
            Box::new(solvers_dto::auction::GyroEPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: {
                    let pool_id_h160: eth::H160 = pool.id.into();
                    pool_id_h160.into()
                },
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::GyroEReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal_v3(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v3::gyro_e::Version::V1 => {
                        solvers_dto::auction::GyroEVersion::V1
                    }
                },
                params_alpha: signed_fixed_point_to_decimal_v3(pool.params_alpha),
                params_beta: signed_fixed_point_to_decimal_v3(pool.params_beta),
                params_c: signed_fixed_point_to_decimal_v3(pool.params_c),
                params_s: signed_fixed_point_to_decimal_v3(pool.params_s),
                params_lambda: signed_fixed_point_to_decimal_v3(pool.params_lambda),
                tau_alpha_x: signed_fixed_point_to_decimal_v3(pool.tau_alpha_x),
                tau_alpha_y: signed_fixed_point_to_decimal_v3(pool.tau_alpha_y),
                tau_beta_x: signed_fixed_point_to_decimal_v3(pool.tau_beta_x),
                tau_beta_y: signed_fixed_point_to_decimal_v3(pool.tau_beta_y),
                u: signed_fixed_point_to_decimal_v3(pool.u),
                v: signed_fixed_point_to_decimal_v3(pool.v),
                w: signed_fixed_point_to_decimal_v3(pool.w),
                z: signed_fixed_point_to_decimal_v3(pool.z),
                d_sq: signed_fixed_point_to_decimal_v3(pool.d_sq),
            }),
        )),

        liquidity::Kind::BalancerV3Gyro2CLP(pool) => Ok(solvers_dto::auction::Liquidity::Gyro2CLP(
            solvers_dto::auction::Gyro2CLPPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: {
                    let pool_id_h160: eth::H160 = pool.id.into();
                    pool_id_h160.into()
                },
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::Gyro2CLPReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal_v3(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v3::gyro_2clp::Version::V1 => {
                        solvers_dto::auction::Gyro2CLPVersion::V1
                    }
                },
                sqrt_alpha: signed_fixed_point_to_decimal_v3_gyro_2clp(pool.sqrt_alpha),
                sqrt_beta: signed_fixed_point_to_decimal_v3_gyro_2clp(pool.sqrt_beta),
            },
        )),

        liquidity::Kind::BalancerV3ReClamm(pool) => Ok(solvers_dto::auction::Liquidity::ReClamm(
            solvers_dto::auction::ReClammPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::ReClammReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal_v3(pool.fee),
                last_virtual_balances: pool
                    .last_virtual_balances
                    .iter()
                    .map(|v| bigdecimal::BigDecimal::new(v.to_big_int(), 0))
                    .collect(),
                daily_price_shift_base: scaling_factor_to_decimal_v3(pool.daily_price_shift_base),
                last_timestamp: pool.last_timestamp,
                centeredness_margin: scaling_factor_to_decimal_v3(pool.centeredness_margin),
                start_fourth_root_price_ratio: scaling_factor_to_decimal_v3(
                    pool.start_fourth_root_price_ratio,
                ),
                end_fourth_root_price_ratio: scaling_factor_to_decimal_v3(
                    pool.end_fourth_root_price_ratio,
                ),
                price_ratio_update_start_time: pool.price_ratio_update_start_time,
                price_ratio_update_end_time: pool.price_ratio_update_end_time,
            },
        )),

        liquidity::Kind::BalancerV3QuantAmm(pool) => Ok(solvers_dto::auction::Liquidity::QuantAmm(
            solvers_dto::auction::QuantAmmPool {
                id: liquidity.id.0.to_string(),
                address: pool.id.address().into(),
                balancer_pool_id: {
                    let pool_id_h160: eth::H160 = pool.id.into();
                    pool_id_h160.into()
                },
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .reserves
                    .iter()
                    .map(|r| {
                        (
                            r.asset.token.into(),
                            solvers_dto::auction::QuantAmmReserve {
                                balance: r.asset.amount.into(),
                                scaling_factor: scaling_factor_to_decimal_v3(r.scale),
                            },
                        )
                    })
                    .collect(),
                fee: fee_to_decimal_v3(pool.fee),
                version: match pool.version {
                    liquidity::balancer::v3::quantamm::Version::V1 => {
                        solvers_dto::auction::QuantAmmVersion::V1
                    }
                },
                max_trade_size_ratio: scaling_factor_to_decimal_v3(pool.max_trade_size_ratio),
                first_four_weights_and_multipliers: pool
                    .first_four_weights_and_multipliers
                    .iter()
                    .map(|i| i256_to_decimal(*i))
                    .collect(),
                second_four_weights_and_multipliers: pool
                    .second_four_weights_and_multipliers
                    .iter()
                    .map(|i| i256_to_decimal(*i))
                    .collect(),
                last_update_time: pool.last_update_time,
                last_interop_time: pool.last_interop_time,
                current_timestamp: pool.current_timestamp,
            },
        )),

        liquidity::Kind::Swapr(pool) => Ok(solvers_dto::auction::Liquidity::ConstantProduct(
            solvers_dto::auction::ConstantProductPool {
                id: liquidity.id.0.to_string(),
                address: pool.base.address.into(),
                router: pool.base.router.into(),
                gas_estimate: liquidity.gas.0.into(),
                tokens: pool
                    .base
                    .reserves
                    .iter()
                    .map(|asset| {
                        (
                            asset.token.into(),
                            solvers_dto::auction::ConstantProductReserve {
                                balance: asset.amount.into(),
                            },
                        )
                    })
                    .collect(),
                fee: bigdecimal::BigDecimal::new(pool.fee.bps().into(), 4),
            },
        )),

        liquidity::Kind::ZeroEx(limit_order) => Ok(solvers_dto::auction::Liquidity::LimitOrder(
            solvers_dto::auction::ForeignLimitOrder {
                id: liquidity.id.0.to_string(),
                address: limit_order.zeroex.address(),
                gas_estimate: liquidity.gas.0.into(),
                hash: Default::default(),
                maker_token: limit_order.order.maker_token,
                taker_token: limit_order.order.taker_token,
                maker_amount: limit_order.fillable.maker.into(),
                taker_amount: limit_order.fillable.taker.into(),
                taker_token_fee_amount: limit_order.order.taker_token_fee_amount.into(),
            },
        )),

        liquidity::Kind::Erc4626(edge) => Ok(solvers_dto::auction::Liquidity::Erc4626(
            solvers_dto::auction::Erc4626Edge {
                id: liquidity.id.0.to_string(),
                gas_estimate: liquidity.gas.0.into(),
                vault: edge.tokens.1.0.into(),
                asset: edge.tokens.0.0.into(),
            },
        )),

        #[allow(unreachable_patterns)]
        _ => {
            tracing::warn!(
                "Unsupported pool type for liquidity conversion: {:?}",
                std::mem::discriminant(&liquidity.kind)
            );
            Err(LiquidityError::UnsupportedPoolType)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LiquidityError {
    #[error("Invalid token pair")]
    InvalidTokenPair,
    #[error("Unsupported pool type")]
    UnsupportedPoolType,
}

fn fee_to_decimal(fee: liquidity::balancer::v2::Fee) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(fee.as_raw().to_big_int(), 18)
}

fn fee_to_decimal_v3(fee: liquidity::balancer::v3::Fee) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(fee.as_raw().to_big_int(), 18)
}

fn weight_to_decimal(weight: liquidity::balancer::v2::weighted::Weight) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(weight.as_raw().to_big_int(), 18)
}

fn weight_to_decimal_v3(
    weight: liquidity::balancer::v3::weighted::Weight,
) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(weight.as_raw().to_big_int(), 18)
}

fn scaling_factor_to_decimal(
    scale: liquidity::balancer::v2::ScalingFactor,
) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(scale.as_raw().to_big_int(), 18)
}

fn scaling_factor_to_decimal_v3(
    scale: liquidity::balancer::v3::ScalingFactor,
) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(scale.as_raw().to_big_int(), 18)
}

fn surge_threshold_to_decimal_v3(
    surge_threshold: liquidity::balancer::v3::stable_surge::SurgeThresholdPercentage,
) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(surge_threshold.value().to_big_int(), 18)
}

fn max_surge_fee_to_decimal_v3(
    max_surge_fee: liquidity::balancer::v3::stable_surge::MaxSurgeFeePercentage,
) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(max_surge_fee.value().to_big_int(), 18)
}

fn signed_fixed_point_to_decimal(
    sfp: liquidity::balancer::v2::gyro_e::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_gyro_2clp(
    sfp: liquidity::balancer::v2::gyro_2clp::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn fixed_point_to_decimal(
    fp: liquidity::balancer::v2::gyro_3clp::FixedPoint,
) -> bigdecimal::BigDecimal {
    let u256_str = fp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(u256_str.as_bytes(), 10)
        .expect("valid U256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_v3(
    sfp: liquidity::balancer::v3::gyro_e::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_v3_gyro_2clp(
    sfp: liquidity::balancer::v3::gyro_2clp::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn i256_to_decimal(i256: ethcontract::I256) -> bigdecimal::BigDecimal {
    let i256_str = i256.to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

impl From<LiquidityError> for (hyper::StatusCode, axum::Json<error::Error>) {
    fn from(error: LiquidityError) -> Self {
        tracing::warn!(?error, "Liquidity API error");

        // Map to existing error kinds that are exposed via the error module
        match error {
            LiquidityError::InvalidTokenPair => {
                // Use the existing From implementation for InvalidTokens error kind
                let auction_error = crate::infra::api::routes::AuctionError::InvalidTokens;
                auction_error.into()
            }
            LiquidityError::UnsupportedPoolType => {
                // For now, just return the same error as InvalidTokens since they both
                // result in Bad Request. We can make this more specific later if needed.
                let auction_error = crate::infra::api::routes::AuctionError::InvalidTokens;
                auction_error.into()
            }
        }
    }
}
