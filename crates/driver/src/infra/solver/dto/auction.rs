use {
    crate::{
        domain::{
            competition::{
                self,
                order::{self, Side, fees, signature::Scheme},
            },
            eth::{self},
            liquidity,
        },
        infra::{config::file::FeeHandler, solver::ManageNativeToken},
        util::conv::{rational_to_big_decimal, u256::U256Ext},
    },
    app_data::AppDataHash,
    model::order::{BuyTokenDestination, SellTokenSource},
    std::collections::HashMap,
};

pub fn new(
    auction: &competition::Auction,
    liquidity: &[liquidity::Liquidity],
    weth: eth::WethAddress,
    fee_handler: FeeHandler,
    solver_native_token: ManageNativeToken,
    flashloan_hints: &HashMap<order::Uid, eth::Flashloan>,
) -> solvers_dto::auction::Auction {
    let mut tokens: HashMap<eth::H160, _> = auction
        .tokens()
        .iter()
        .map(|token| {
            (
                token.address.into(),
                solvers_dto::auction::Token {
                    decimals: token.decimals,
                    symbol: token.symbol.clone(),
                    reference_price: token.price.map(Into::into),
                    available_balance: token.available_balance,
                    trusted: token.trusted,
                },
            )
        })
        .collect();

    // Make sure that we have at least empty entries for all tokens for
    // which we are providing liquidity.
    for token in liquidity
        .iter()
        .flat_map(|liquidity| match &liquidity.kind {
            liquidity::Kind::UniswapV2(pool) => pool.reserves.iter().map(|r| r.token).collect(),
            liquidity::Kind::UniswapV3(pool) => vec![pool.tokens.get().0, pool.tokens.get().1],
            liquidity::Kind::BalancerV2Stable(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3Stable(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3StableSurge(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV2Weighted(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3Weighted(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV2GyroE(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV2Gyro2CLP(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV2Gyro3CLP(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3GyroE(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3Gyro2CLP(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3ReClamm(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::BalancerV3QuantAmm(pool) => pool.reserves.tokens().collect(),
            liquidity::Kind::Swapr(pool) => pool.base.reserves.iter().map(|r| r.token).collect(),
            liquidity::Kind::ZeroEx(limit_order) => {
                vec![
                    limit_order.order.maker_token.into(),
                    limit_order.order.taker_token.into(),
                ]
            }
            liquidity::Kind::Erc4626(edge) => vec![edge.tokens.0, edge.tokens.1],
        })
    {
        tokens.entry(token.into()).or_insert_with(Default::default);
    }

    solvers_dto::auction::Auction {
        id: auction.id().as_ref().map(|id| id.0),
        orders: auction
            .orders()
            .iter()
            .map(|order| {
                let mut available = order.available();

                if solver_native_token.wrap_address {
                    available.buy.token = available.buy.token.as_erc20(weth)
                }
                // In case of volume based fees, fee withheld by driver might be higher than the
                // surplus of the solution. This would lead to violating limit prices when
                // driver tries to withhold the volume based fee. To avoid this, we artificially
                // adjust the order limit amounts (make then worse) before sending to solvers,
                // to force solvers to only submit solutions with enough surplus to cover the
                // fee.
                //
                // https://github.com/cowprotocol/services/issues/2440
                if fee_handler == FeeHandler::Driver {
                    order.protocol_fees.iter().for_each(|protocol_fee| {
                        if let fees::FeePolicy::Volume { factor } = protocol_fee {
                            match order.side {
                                Side::Buy => {
                                    // reduce sell amount by factor
                                    available.sell.amount = available
                                        .sell
                                        .amount
                                        .apply_factor(1.0 / (1.0 + factor))
                                        .unwrap_or_default();
                                }
                                Side::Sell => {
                                    // increase buy amount by factor
                                    available.buy.amount = available
                                        .buy
                                        .amount
                                        .apply_factor(1.0 / (1.0 - factor))
                                        .unwrap_or_default();
                                }
                            }
                        }
                    })
                }
                solvers_dto::auction::Order {
                    uid: order.uid.into(),
                    sell_token: available.sell.token.into(),
                    buy_token: available.buy.token.into(),
                    sell_amount: available.sell.amount.into(),
                    buy_amount: available.buy.amount.into(),
                    full_sell_amount: order.sell.amount.into(),
                    full_buy_amount: order.buy.amount.into(),
                    kind: match order.side {
                        Side::Buy => solvers_dto::auction::Kind::Buy,
                        Side::Sell => solvers_dto::auction::Kind::Sell,
                    },
                    receiver: order.receiver.map(Into::into),
                    owner: order.signature.signer.into(),
                    partially_fillable: order.is_partial(),
                    class: match order.kind {
                        order::Kind::Market => solvers_dto::auction::Class::Market,
                        order::Kind::Limit => solvers_dto::auction::Class::Limit,
                    },
                    pre_interactions: order
                        .pre_interactions
                        .iter()
                        .cloned()
                        .map(interaction_from_domain)
                        .collect::<Vec<_>>(),
                    post_interactions: order
                        .post_interactions
                        .iter()
                        .cloned()
                        .map(interaction_from_domain)
                        .collect::<Vec<_>>(),
                    sell_token_source: sell_token_source_from_domain(
                        order.sell_token_balance.into(),
                    ),
                    buy_token_destination: buy_token_destination_from_domain(
                        order.buy_token_balance.into(),
                    ),
                    fee_policies: (fee_handler == FeeHandler::Solver).then_some(
                        order
                            .protocol_fees
                            .iter()
                            .cloned()
                            .map(fee_policy_from_domain)
                            .collect(),
                    ),
                    app_data: AppDataHash(order.app_data.hash().0.into()),
                    flashloan_hint: flashloan_hints.get(&order.uid).map(Into::into),
                    signature: order.signature.data.clone().into(),
                    signing_scheme: match order.signature.scheme {
                        Scheme::Eip712 => solvers_dto::auction::SigningScheme::Eip712,
                        Scheme::EthSign => solvers_dto::auction::SigningScheme::EthSign,
                        Scheme::Eip1271 => solvers_dto::auction::SigningScheme::Eip1271,
                        Scheme::PreSign => solvers_dto::auction::SigningScheme::PreSign,
                    },
                    valid_to: order.valid_to.into(),
                }
            })
            .collect(),
        liquidity: liquidity
            .iter()
            .filter_map(|liquidity| {
                Some(match &liquidity.kind {
                    liquidity::Kind::UniswapV2(pool) => {
                        solvers_dto::auction::Liquidity::ConstantProduct(
                            solvers_dto::auction::ConstantProductPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.address.into(),
                                router: pool.router.into(),
                                gas_estimate: liquidity.gas.into(),
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
                        )
                    }
                    liquidity::Kind::UniswapV3(pool) => {
                        solvers_dto::auction::Liquidity::ConcentratedLiquidity(
                            solvers_dto::auction::ConcentratedLiquidityPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.address.0,
                                router: pool.router.into(),
                                gas_estimate: liquidity.gas.0,
                                tokens: vec![
                                    pool.tokens.get().0.into(),
                                    pool.tokens.get().1.into(),
                                ],
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
                        )
                    }
                    liquidity::Kind::BalancerV2Stable(pool) => {
                        solvers_dto::auction::Liquidity::Stable(solvers_dto::auction::StablePool {
                            id: liquidity.id.0.to_string(),
                            address: pool.id.address().into(),
                            balancer_pool_id: pool.id.into(),
                            gas_estimate: liquidity.gas.into(),
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
                            amplification_parameter: rational_to_big_decimal(
                                &num::BigRational::new(
                                    pool.amplification_parameter.factor().to_big_int(),
                                    pool.amplification_parameter.precision().to_big_int(),
                                ),
                            ),
                            fee: fee_to_decimal(pool.fee),
                        })
                    }
                    liquidity::Kind::BalancerV3Stable(pool) => {
                        solvers_dto::auction::Liquidity::Stable(solvers_dto::auction::StablePool {
                            id: liquidity.id.0.to_string(),
                            address: pool.id.address().into(),
                            balancer_pool_id: {
                                let pool_id_h160: eth::H160 = pool.id.into();
                                pool_id_h160.into()
                            },
                            gas_estimate: liquidity.gas.into(),
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
                            amplification_parameter: rational_to_big_decimal(
                                &num::BigRational::new(
                                    pool.amplification_parameter.factor().to_big_int(),
                                    pool.amplification_parameter.precision().to_big_int(),
                                ),
                            ),
                            fee: fee_to_decimal_v3(pool.fee),
                        })
                    }
                    liquidity::Kind::BalancerV2Weighted(pool) => {
                        solvers_dto::auction::Liquidity::WeightedProduct(
                            solvers_dto::auction::WeightedProductPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: pool.id.into(),
                                gas_estimate: liquidity.gas.into(),
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
                        )
                    }
                    liquidity::Kind::BalancerV3Weighted(pool) => {
                        solvers_dto::auction::Liquidity::WeightedProduct(
                            solvers_dto::auction::WeightedProductPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: {
                                    let pool_id_h160: eth::H160 = pool.id.into();
                                    pool_id_h160.into()
                                },
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::WeightedProductReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
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
                                    } /* Future versions can be added here:
                                       * liquidity::balancer::v3::weighted::Version::V2 => {
                                       *     solvers_dto::auction::WeightedProductVersion::V2
                                       * } */
                                },
                            },
                        )
                    }
                    liquidity::Kind::BalancerV2GyroE(pool) => {
                        solvers_dto::auction::Liquidity::GyroE(Box::new(
                            solvers_dto::auction::GyroEPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: pool.id.into(),
                                gas_estimate: liquidity.gas.into(),
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
                                // Convert all Gyro E-CLP static parameters to BigDecimal
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
                            },
                        ))
                    }
                    liquidity::Kind::BalancerV2Gyro2CLP(pool) => {
                        solvers_dto::auction::Liquidity::Gyro2CLP(
                            solvers_dto::auction::Gyro2CLPPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: pool.id.into(),
                                gas_estimate: liquidity.gas.into(),
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
                                // Convert Gyro 2-CLP static parameters to BigDecimal
                                sqrt_alpha: signed_fixed_point_to_decimal_gyro_2clp(
                                    pool.sqrt_alpha,
                                ),
                                sqrt_beta: signed_fixed_point_to_decimal_gyro_2clp(pool.sqrt_beta),
                            },
                        )
                    }
                    liquidity::Kind::BalancerV2Gyro3CLP(pool) => {
                        solvers_dto::auction::Liquidity::Gyro3CLP(
                            solvers_dto::auction::Gyro3CLPPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: pool.id.into(),
                                gas_estimate: liquidity.gas.into(),
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
                                // Convert Gyro 3-CLP static parameter to BigDecimal
                                root3_alpha: fixed_point_to_decimal(pool.root3_alpha),
                            },
                        )
                    }
                    liquidity::Kind::BalancerV3GyroE(pool) => {
                        solvers_dto::auction::Liquidity::GyroE(Box::new(
                            solvers_dto::auction::GyroEPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: {
                                    let pool_id_h160: eth::H160 = pool.id.into();
                                    pool_id_h160.into()
                                },
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::GyroEReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
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
                                // Convert all Gyro E-CLP static parameters to BigDecimal
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
                            },
                        ))
                    }
                    liquidity::Kind::BalancerV3Gyro2CLP(pool) => {
                        solvers_dto::auction::Liquidity::Gyro2CLP(
                            solvers_dto::auction::Gyro2CLPPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: {
                                    let pool_id_h160: eth::H160 = pool.id.into();
                                    pool_id_h160.into()
                                },
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::Gyro2CLPReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
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
                                // Convert Gyro 2-CLP static parameters to BigDecimal
                                sqrt_alpha: signed_fixed_point_to_decimal_v3_gyro_2clp(
                                    pool.sqrt_alpha,
                                ),
                                sqrt_beta: signed_fixed_point_to_decimal_v3_gyro_2clp(
                                    pool.sqrt_beta,
                                ),
                            },
                        )
                    }
                    liquidity::Kind::BalancerV3ReClamm(pool) => {
                        solvers_dto::auction::Liquidity::ReClamm(
                            solvers_dto::auction::ReClammPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::ReClammReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
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
                                daily_price_shift_base: scaling_factor_to_decimal_v3(
                                    pool.daily_price_shift_base,
                                ),
                                last_timestamp: pool.last_timestamp,
                                centeredness_margin: scaling_factor_to_decimal_v3(
                                    pool.centeredness_margin,
                                ),
                                start_fourth_root_price_ratio: scaling_factor_to_decimal_v3(
                                    pool.start_fourth_root_price_ratio,
                                ),
                                end_fourth_root_price_ratio: scaling_factor_to_decimal_v3(
                                    pool.end_fourth_root_price_ratio,
                                ),
                                price_ratio_update_start_time: pool.price_ratio_update_start_time,
                                price_ratio_update_end_time: pool.price_ratio_update_end_time,
                            },
                        )
                    }
                    liquidity::Kind::BalancerV3QuantAmm(pool) => {
                        solvers_dto::auction::Liquidity::QuantAmm(
                            solvers_dto::auction::QuantAmmPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: {
                                    let pool_id_h160: eth::H160 = pool.id.into();
                                    pool_id_h160.into()
                                },
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::QuantAmmReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
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
                                max_trade_size_ratio: scaling_factor_to_decimal_v3(
                                    pool.max_trade_size_ratio,
                                ),
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
                        )
                    }
                    liquidity::Kind::Swapr(pool) => {
                        solvers_dto::auction::Liquidity::ConstantProduct(
                            solvers_dto::auction::ConstantProductPool {
                                id: liquidity.id.0.to_string(),
                                address: pool.base.address.into(),
                                router: pool.base.router.into(),
                                gas_estimate: liquidity.gas.into(),
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
                        )
                    }
                    liquidity::Kind::ZeroEx(limit_order) => {
                        solvers_dto::auction::Liquidity::LimitOrder(
                            solvers_dto::auction::ForeignLimitOrder {
                                id: liquidity.id.0.to_string(),
                                address: limit_order.zeroex.address(),
                                gas_estimate: liquidity.gas.into(),
                                hash: Default::default(),
                                maker_token: limit_order.order.maker_token,
                                taker_token: limit_order.order.taker_token,
                                maker_amount: limit_order.fillable.maker.into(),
                                taker_amount: limit_order.fillable.taker.into(),
                                taker_token_fee_amount: limit_order
                                    .order
                                    .taker_token_fee_amount
                                    .into(),
                            },
                        )
                    }
                    liquidity::Kind::Erc4626(edge) => {
                        // Expose a minimal ERC4626 edge to external solvers.
                        // Add a new DTO variant and map it here.
                        solvers_dto::auction::Liquidity::Erc4626(
                            solvers_dto::auction::Erc4626Edge {
                                id: liquidity.id.0.to_string(),
                                gas_estimate: liquidity.gas.into(),
                                vault: edge.tokens.1.0.into(),
                                asset: edge.tokens.0.0.into(),
                            },
                        )
                    }
                    liquidity::Kind::BalancerV3StableSurge(pool) => {
                        // StableSurge pools use dynamic fees based on pool imbalance
                        // External solvers receive surge parameters to make informed routing
                        // decisions
                        solvers_dto::auction::Liquidity::StableSurge(
                            solvers_dto::auction::StableSurgePool {
                                id: liquidity.id.0.to_string(),
                                address: pool.id.address().into(),
                                balancer_pool_id: {
                                    let pool_id_h160: eth::H160 = pool.id.into();
                                    pool_id_h160.into()
                                },
                                gas_estimate: liquidity.gas.into(),
                                tokens: pool
                                    .reserves
                                    .iter()
                                    .map(|r| {
                                        (
                                            r.asset.token.into(),
                                            solvers_dto::auction::StableReserve {
                                                balance: r.asset.amount.into(),
                                                scaling_factor: scaling_factor_to_decimal_v3(
                                                    r.scale,
                                                ),
                                            },
                                        )
                                    })
                                    .collect(),
                                amplification_parameter: rational_to_big_decimal(
                                    &num::BigRational::new(
                                        pool.amplification_parameter.factor().to_big_int(),
                                        pool.amplification_parameter.precision().to_big_int(),
                                    ),
                                ),
                                fee: fee_to_decimal_v3(pool.fee),
                                surge_threshold_percentage: surge_threshold_to_decimal_v3(
                                    pool.surge_threshold_percentage.clone(),
                                ),
                                max_surge_fee_percentage: max_surge_fee_to_decimal_v3(
                                    pool.max_surge_fee_percentage.clone(),
                                ),
                            },
                        )
                    }
                })
            })
            .collect(),
        tokens,
        effective_gas_price: auction.gas_price().effective().into(),
        deadline: auction.deadline().solvers(),
        surplus_capturing_jit_order_owners: auction
            .surplus_capturing_jit_order_owners()
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<_>>(),
    }
}

fn fee_policy_from_domain(value: fees::FeePolicy) -> solvers_dto::auction::FeePolicy {
    match value {
        order::FeePolicy::Surplus {
            factor,
            max_volume_factor,
        } => solvers_dto::auction::FeePolicy::Surplus {
            factor,
            max_volume_factor,
        },
        order::FeePolicy::PriceImprovement {
            factor,
            max_volume_factor,
            quote,
        } => solvers_dto::auction::FeePolicy::PriceImprovement {
            factor,
            max_volume_factor,
            quote: solvers_dto::auction::Quote {
                sell_amount: quote.sell.amount.into(),
                buy_amount: quote.buy.amount.into(),
                fee: quote.fee.amount.into(),
            },
        },
        order::FeePolicy::Volume { factor } => solvers_dto::auction::FeePolicy::Volume { factor },
    }
}

fn interaction_from_domain(value: eth::Interaction) -> solvers_dto::auction::InteractionData {
    solvers_dto::auction::InteractionData {
        target: value.target.0,
        value: value.value.0,
        call_data: value.call_data.0,
    }
}

fn sell_token_source_from_domain(value: SellTokenSource) -> solvers_dto::auction::SellTokenSource {
    match value {
        SellTokenSource::Erc20 => solvers_dto::auction::SellTokenSource::Erc20,
        SellTokenSource::External => solvers_dto::auction::SellTokenSource::External,
        SellTokenSource::Internal => solvers_dto::auction::SellTokenSource::Internal,
    }
}

fn buy_token_destination_from_domain(
    value: BuyTokenDestination,
) -> solvers_dto::auction::BuyTokenDestination {
    match value {
        BuyTokenDestination::Erc20 => solvers_dto::auction::BuyTokenDestination::Erc20,
        BuyTokenDestination::Internal => solvers_dto::auction::BuyTokenDestination::Internal,
    }
}

fn i256_to_decimal(i256: ethcontract::I256) -> bigdecimal::BigDecimal {
    let i256_str = i256.to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn fee_to_decimal(fee: liquidity::balancer::v2::Fee) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(fee.as_raw().to_big_int(), 18)
}

fn fee_to_decimal_v3(fee: liquidity::balancer::v3::Fee) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::new(fee.as_raw().to_big_int(), 18)
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

fn signed_fixed_point_to_decimal(
    sfp: liquidity::balancer::v2::gyro_e::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    // Convert I256 to BigInt via string representation to handle signed values
    // correctly
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_gyro_2clp(
    sfp: liquidity::balancer::v2::gyro_2clp::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    // Convert I256 to BigInt via string representation to handle signed values
    // correctly
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn fixed_point_to_decimal(
    fp: liquidity::balancer::v2::gyro_3clp::FixedPoint,
) -> bigdecimal::BigDecimal {
    // Convert U256 to BigInt via string representation
    let u256_str = fp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(u256_str.as_bytes(), 10)
        .expect("valid U256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_v3(
    sfp: liquidity::balancer::v3::gyro_e::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    // Convert I256 to BigInt via string representation to handle signed values
    // correctly
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}

fn signed_fixed_point_to_decimal_v3_gyro_2clp(
    sfp: liquidity::balancer::v3::gyro_2clp::SignedFixedPoint,
) -> bigdecimal::BigDecimal {
    // Convert I256 to BigInt via string representation to handle signed values
    // correctly
    let i256_str = sfp.as_raw().to_string();
    let big_int = num::BigInt::parse_bytes(i256_str.as_bytes(), 10)
        .expect("valid I256 should parse to BigInt");
    bigdecimal::BigDecimal::new(big_int, 18)
}
