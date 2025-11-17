//! Module for providing Balancer V3 pool liquidity to the solvers.

use {
    crate::{
        interactions::{
            BalancerV3SwapGivenOutInteraction,
            allowances::{AllowanceManager, AllowanceManaging, Allowances},
        },
        liquidity::{
            AmmOrderExecution,
            BalancerV3Gyro2CLPOrder,
            BalancerV3GyroEOrder,
            BalancerV3QuantAmmOrder,
            BalancerV3ReClammOrder,
            BalancerV3StablePoolOrder,
            BalancerV3StableSurgePoolOrder,
            BalancerV3WeightedProductOrder,
            Liquidity,
            SettlementHandling,
        },
        liquidity_collector::LiquidityCollecting,
        settlement::SettlementEncoder,
    },
    anyhow::Result,
    contracts::{GPv2Settlement, alloy::BalancerV3BatchRouter},
    ethcontract::H160,
    ethrpc::alloy::conversions::IntoLegacy,
    model::TokenPair,
    shared::{
        ethrpc::Web3,
        http_solver::model::TokenAmount,
        recent_block_cache::Block,
        sources::balancer_v3::pool_fetching::BalancerV3PoolFetching,
    },
    std::{collections::HashSet, sync::Arc},
};

/// A liquidity provider for Balancer V3 weighted pools.
pub struct BalancerV3Liquidity {
    settlement: GPv2Settlement,
    batch_router: BalancerV3BatchRouter::Instance,
    pool_fetcher: Arc<dyn BalancerV3PoolFetching>,
    allowance_manager: Box<dyn AllowanceManaging>,
}

impl BalancerV3Liquidity {
    pub fn new(
        web3: Web3,
        pool_fetcher: Arc<dyn BalancerV3PoolFetching>,
        settlement: GPv2Settlement,
        batch_router: BalancerV3BatchRouter::Instance,
    ) -> Self {
        let allowance_manager = AllowanceManager::new(web3, settlement.address());
        Self {
            settlement,
            batch_router,
            pool_fetcher,
            allowance_manager: Box::new(allowance_manager),
        }
    }

    async fn get_orders(
        &self,
        pairs: HashSet<TokenPair>,
        block: Block,
    ) -> Result<(
        Vec<BalancerV3StablePoolOrder>,
        Vec<BalancerV3StableSurgePoolOrder>,
        Vec<BalancerV3WeightedProductOrder>,
        Vec<BalancerV3GyroEOrder>,
        Vec<BalancerV3Gyro2CLPOrder>,
        Vec<BalancerV3ReClammOrder>,
        Vec<BalancerV3QuantAmmOrder>,
    )> {
        let pools = self.pool_fetcher.fetch(pairs, block).await?;

        let tokens = pools.relevant_tokens();

        let allowances = self
            .allowance_manager
            .get_allowances(tokens, self.batch_router.address().into_legacy())
            .await?;

        let inner = Arc::new(Inner {
            allowances,
            settlement: self.settlement.clone(),
            batch_router: self.batch_router.clone(),
        });

        let weighted_product_orders: Vec<_> = pools
            .weighted_pools
            .into_iter()
            .map(|pool| BalancerV3WeightedProductOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                version: pool.version,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        let stable_pool_orders: Vec<_> = pools
            .stable_pools
            .into_iter()
            .map(|pool| BalancerV3StablePoolOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                amplification_parameter: pool.amplification_parameter,
                version: pool.version,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        let stable_surge_pool_orders: Vec<_> = pools
            .stable_surge_pools
            .into_iter()
            .map(|pool| BalancerV3StableSurgePoolOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                amplification_parameter: pool.amplification_parameter,
                version: pool.version,
                surge_threshold_percentage: pool.surge_threshold_percentage,
                max_surge_fee_percentage: pool.max_surge_fee_percentage,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        let gyro_e_orders: Vec<_> = pools
            .gyro_e_pools
            .into_iter()
            .map(|pool| BalancerV3GyroEOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                version: pool.version,
                params_alpha: pool.params_alpha,
                params_beta: pool.params_beta,
                params_c: pool.params_c,
                params_s: pool.params_s,
                params_lambda: pool.params_lambda,
                tau_alpha_x: pool.tau_alpha_x,
                tau_alpha_y: pool.tau_alpha_y,
                tau_beta_x: pool.tau_beta_x,
                tau_beta_y: pool.tau_beta_y,
                u: pool.u,
                v: pool.v,
                w: pool.w,
                z: pool.z,
                d_sq: pool.d_sq,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        let gyro_2clp_orders: Vec<_> = pools
            .gyro_2clp_pools
            .into_iter()
            .map(|pool| BalancerV3Gyro2CLPOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                version: pool.version,
                sqrt_alpha: pool.sqrt_alpha,
                sqrt_beta: pool.sqrt_beta,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        let reclamm_orders: Vec<_> = pools
            .reclamm_pools
            .into_iter()
            .map(|pool| BalancerV3ReClammOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                version: pool.version,
                last_virtual_balances: pool.last_virtual_balances.into_iter().collect(),
                daily_price_shift_base: pool.daily_price_shift_base,
                last_timestamp: pool.last_timestamp,
                centeredness_margin: pool.centeredness_margin,
                start_fourth_root_price_ratio: pool.start_fourth_root_price_ratio,
                end_fourth_root_price_ratio: pool.end_fourth_root_price_ratio,
                price_ratio_update_start_time: pool.price_ratio_update_start_time,
                price_ratio_update_end_time: pool.price_ratio_update_end_time,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();
        let quantamm_orders: Vec<_> = pools
            .quantamm_pools
            .into_iter()
            .map(|pool| BalancerV3QuantAmmOrder {
                address: pool.common.address,
                reserves: pool.reserves,
                fee: pool.common.swap_fee,
                version: pool.version,
                max_trade_size_ratio: pool.max_trade_size_ratio,
                first_four_weights_and_multipliers: pool.first_four_weights_and_multipliers,
                second_four_weights_and_multipliers: pool.second_four_weights_and_multipliers,
                last_update_time: pool.last_update_time,
                last_interop_time: pool.last_interop_time,
                current_timestamp: pool.current_timestamp,
                settlement_handling: Arc::new(SettlementHandler {
                    pool_id: pool.common.id,
                    inner: inner.clone(),
                }),
            })
            .collect();

        Ok((
            stable_pool_orders,
            stable_surge_pool_orders,
            weighted_product_orders,
            gyro_e_orders,
            gyro_2clp_orders,
            reclamm_orders,
            quantamm_orders,
        ))
    }
}

#[async_trait::async_trait]
impl LiquidityCollecting for BalancerV3Liquidity {
    /// Returns relevant Balancer V3 weighted pools given a list of off-chain
    /// orders.
    async fn get_liquidity(
        &self,
        pairs: HashSet<TokenPair>,
        block: Block,
    ) -> Result<Vec<Liquidity>> {
        let (stable, stable_surge, weighted, gyro_e, gyro_2clp, reclamm, quantamm) =
            self.get_orders(pairs, block).await?;
        let liquidity = stable
            .into_iter()
            .map(Liquidity::BalancerV3Stable)
            .chain(
                stable_surge
                    .into_iter()
                    .map(Liquidity::BalancerV3StableSurge),
            )
            .chain(weighted.into_iter().map(Liquidity::BalancerV3Weighted))
            .chain(gyro_e.into_iter().map(Liquidity::BalancerV3GyroE))
            .chain(gyro_2clp.into_iter().map(Liquidity::BalancerV3Gyro2CLP))
            .chain(reclamm.into_iter().map(Liquidity::BalancerV3ReClamm))
            .chain(quantamm.into_iter().map(Liquidity::BalancerV3QuantAmm))
            .collect();
        Ok(liquidity)
    }
}

pub struct SettlementHandler {
    pool_id: H160,
    inner: Arc<Inner>,
}

struct Inner {
    settlement: GPv2Settlement,
    batch_router: BalancerV3BatchRouter::Instance,
    allowances: Allowances,
}

impl SettlementHandler {
    pub fn new(
        pool_id: H160,
        settlement: GPv2Settlement,
        batch_router: BalancerV3BatchRouter::Instance,
        allowances: Allowances,
    ) -> Self {
        SettlementHandler {
            pool_id,
            inner: Arc::new(Inner {
                settlement,
                batch_router,
                allowances,
            }),
        }
    }

    pub fn batch_router(&self) -> &BalancerV3BatchRouter::Instance {
        &self.inner.batch_router
    }

    pub fn pool_id(&self) -> H160 {
        self.pool_id
    }

    pub fn swap(
        &self,
        input_max: TokenAmount,
        output: TokenAmount,
    ) -> BalancerV3SwapGivenOutInteraction {
        BalancerV3SwapGivenOutInteraction {
            settlement: self.inner.settlement.clone(),
            batch_router: self.inner.batch_router.clone(),
            pool: self.pool_id,
            asset_in_max: input_max,
            asset_out: output,
            // Balancer V3 pools allow passing additional user data in order to
            // control pool behaviour for swaps. That being said, weighted pools
            // do not seem to make use of this at the moment so leave it empty.
            user_data: Default::default(),
        }
    }
}

impl SettlementHandling<BalancerV3WeightedProductOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3StablePoolOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3StableSurgePoolOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3GyroEOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3Gyro2CLPOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3ReClammOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandling<BalancerV3QuantAmmOrder> for SettlementHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: AmmOrderExecution, encoder: &mut SettlementEncoder) -> Result<()> {
        self.inner_encode(execution, encoder)
    }
}

impl SettlementHandler {
    fn inner_encode(
        &self,
        execution: AmmOrderExecution,
        encoder: &mut SettlementEncoder,
    ) -> Result<()> {
        if let Some(approval) = self
            .inner
            .allowances
            .approve_token(execution.input_max.clone())?
        {
            encoder.append_to_execution_plan_internalizable(
                Arc::new(approval),
                execution.internalizable,
            );
        }
        encoder.append_to_execution_plan_internalizable(
            Arc::new(self.swap(execution.input_max, execution.output)),
            execution.internalizable,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::interactions::allowances::{Approval, MockAllowanceManaging},
        contracts::dummy_contract,
        maplit::{btreemap, hashmap, hashset},
        mockall::predicate::*,
        model::TokenPair,
        primitive_types::{H160, U256},
        shared::{
            baseline_solver::BaseTokens,
            http_solver::model::{InternalizationStrategy, TokenAmount},
            interaction::Interaction,
            sources::balancer_v3::{
                pool_fetching::{
                    CommonPoolState,
                    FetchedBalancerPools,
                    MockBalancerV3PoolFetching,
                    WeightedPool,
                    WeightedPoolVersion,
                    WeightedTokenState,
                },
                swap::fixed_point::Bfp as V3Bfp,
            },
        },
    };

    fn dummy_contracts() -> (GPv2Settlement, BalancerV3BatchRouter::Instance) {
        (
            dummy_contract!(GPv2Settlement, H160([0xc0; 20])),
            BalancerV3BatchRouter::Instance::new([0xc1; 20].into(), ethrpc::mock::web3().alloy),
        )
    }

    fn token_pair(seed0: u8, seed1: u8) -> TokenPair {
        TokenPair::new(H160([seed0; 20]), H160([seed1; 20])).unwrap()
    }

    #[tokio::test]
    async fn fetches_liquidity() {
        let mut pool_fetcher = MockBalancerV3PoolFetching::new();
        let mut allowance_manager = MockAllowanceManaging::new();

        let weighted_pools = vec![
            WeightedPool {
                common: CommonPoolState {
                    id: H160([0x90; 20]),
                    address: H160([0x90; 20]),
                    swap_fee: "0.002".parse().unwrap(),
                    paused: true,
                },
                reserves: btreemap! {
                    H160([0x70; 20]) => WeightedTokenState {
                        common: shared::sources::balancer_v3::pool_fetching::TokenState {
                            balance: 100.into(),
                            scaling_factor: V3Bfp::exp10(16),
                            rate: U256::exp10(18),
                        },
                        weight: "0.25".parse().unwrap(),
                    },
                    H160([0x71; 20]) => WeightedTokenState {
                        common: shared::sources::balancer_v3::pool_fetching::TokenState {
                            balance: 1_000_000.into(),
                            scaling_factor: V3Bfp::exp10(12),
                            rate: U256::exp10(18),
                        },
                        weight: "0.25".parse().unwrap(),
                    },
                    H160([0xb0; 20]) => WeightedTokenState {
                        common: shared::sources::balancer_v3::pool_fetching::TokenState {
                            balance: 1_000_000_000_000_000_000u128.into(),
                            scaling_factor: V3Bfp::exp10(0),
                            rate: U256::exp10(18),
                        },
                        weight: "0.5".parse().unwrap(),
                    },
                },
                version: WeightedPoolVersion::V1,
            },
            WeightedPool {
                common: CommonPoolState {
                    id: H160([0x91; 20]),
                    address: H160([0x91; 20]),
                    swap_fee: "0.001".parse().unwrap(),
                    paused: true,
                },
                reserves: btreemap! {
                    H160([0x73; 20]) => WeightedTokenState {
                        common: shared::sources::balancer_v3::pool_fetching::TokenState {
                            balance: 1_000_000_000_000_000_000u128.into(),
                            scaling_factor: V3Bfp::exp10(0),
                            rate: U256::exp10(18),
                        },
                        weight: "0.5".parse().unwrap(),
                    },
                    H160([0xb0; 20]) => WeightedTokenState {
                        common: shared::sources::balancer_v3::pool_fetching::TokenState {
                            balance: 1_000_000_000_000_000_000u128.into(),
                            scaling_factor: V3Bfp::exp10(0),
                            rate: U256::exp10(18),
                        },
                        weight: "0.5".parse().unwrap(),
                    },
                },
                version: WeightedPoolVersion::V1,
            },
        ];

        // Fetches pools for all relevant tokens, in this example, there is no
        // pool for token 0x72..72.
        pool_fetcher
            .expect_fetch()
            .with(
                eq(hashset![
                    token_pair(0x70, 0x71),
                    token_pair(0x70, 0xb0),
                    token_pair(0xb0, 0x71),
                    token_pair(0x70, 0x72),
                    token_pair(0xb0, 0x72),
                    token_pair(0xb0, 0x73),
                ]),
                always(),
            )
            .returning({
                let weighted_pools = weighted_pools.clone();
                move |_, _| {
                    Ok(FetchedBalancerPools {
                        weighted_pools: weighted_pools.clone(),
                        stable_pools: vec![],
                        stable_surge_pools: vec![],
                        gyro_2clp_pools: vec![],
                        gyro_e_pools: vec![],
                        reclamm_pools: vec![],
                        quantamm_pools: vec![],
                    })
                }
            });

        // Fetches allowances for all tokens in pools.
        allowance_manager
            .expect_get_allowances()
            .with(
                eq(hashset![
                    H160([0x70; 20]),
                    H160([0x71; 20]),
                    H160([0x73; 20]),
                    H160([0xb0; 20]),
                ]),
                eq(H160([0xc1; 20])),
            )
            .returning(|_, _| Ok(Allowances::empty(H160([0xc1; 20]))));

        let base_tokens = BaseTokens::new(H160([0xb0; 20]), &[]);
        let traded_pairs = [
            TokenPair::new(H160([0x70; 20]), H160([0x71; 20])).unwrap(),
            TokenPair::new(H160([0x70; 20]), H160([0x72; 20])).unwrap(),
            TokenPair::new(H160([0xb0; 20]), H160([0x73; 20])).unwrap(),
        ];
        let pairs = base_tokens.relevant_pairs(traded_pairs.into_iter());

        let (settlement, batch_router) = dummy_contracts();
        let liquidity_provider = BalancerV3Liquidity {
            settlement,
            batch_router,
            pool_fetcher: Arc::new(pool_fetcher),
            allowance_manager: Box::new(allowance_manager),
        };
        let (
            _stable_orders,
            _stable_surge_orders,
            weighted_orders,
            _gyro_e_orders,
            _gyro_2clp_orders,
            _reclamm_orders,
            _quantamm_orders,
        ) = liquidity_provider
            .get_orders(pairs, Block::Recent)
            .await
            .unwrap();

        assert_eq!(weighted_orders.len(), 2);

        assert_eq!(
            (
                &weighted_orders[0].reserves,
                &weighted_orders[0].fee,
                weighted_orders[0].version
            ),
            (
                &weighted_pools[0].reserves,
                &"0.002".parse().unwrap(),
                WeightedPoolVersion::V1
            ),
        );
        assert_eq!(
            (
                &weighted_orders[1].reserves,
                &weighted_orders[1].fee,
                weighted_orders[1].version
            ),
            (
                &weighted_pools[1].reserves,
                &"0.001".parse().unwrap(),
                WeightedPoolVersion::V1
            ),
        );
    }

    #[tokio::test]
    async fn fetches_reclamm_liquidity() {
        let mut pool_fetcher = MockBalancerV3PoolFetching::new();
        let mut allowance_manager = MockAllowanceManaging::new();

        let token_a = H160([0xaa; 20]);
        let token_b = H160([0xbb; 20]);
        let reclamm_pools = vec![shared::sources::balancer_v3::pool_fetching::ReClammPool {
            common: CommonPoolState {
                id: H160([0x95; 20]),
                address: H160([0x95; 20]),
                swap_fee: "0.003".parse().unwrap(),
                paused: false,
            },
            reserves: btreemap! {
                token_a => shared::sources::balancer_v3::pool_fetching::TokenState {
                    balance: 1_000_000u128.into(),
                    scaling_factor: V3Bfp::exp10(0),
                    rate: U256::exp10(18),
                },
                token_b => shared::sources::balancer_v3::pool_fetching::TokenState {
                    balance: 2_000_000u128.into(),
                    scaling_factor: V3Bfp::exp10(0),
                    rate: U256::exp10(18),
                },
            },
            version: shared::sources::balancer_v3::pools::reclamm::Version::V2,
            last_virtual_balances: vec![10u64.into(), 20u64.into()],
            daily_price_shift_base: "1".parse().unwrap(),
            last_timestamp: 1,
            centeredness_margin: "0.5".parse().unwrap(),
            start_fourth_root_price_ratio: "1".parse().unwrap(),
            end_fourth_root_price_ratio: "1".parse().unwrap(),
            price_ratio_update_start_time: 0,
            price_ratio_update_end_time: 0,
        }];

        pool_fetcher
            .expect_fetch()
            .with(always(), always())
            .returning({
                let reclamm_pools = reclamm_pools.clone();
                move |_, _| {
                    Ok(FetchedBalancerPools {
                        weighted_pools: vec![],
                        stable_pools: vec![],
                        stable_surge_pools: vec![],
                        gyro_2clp_pools: vec![],
                        gyro_e_pools: vec![],
                        reclamm_pools: reclamm_pools.clone(),
                        quantamm_pools: vec![],
                    })
                }
            });

        allowance_manager
            .expect_get_allowances()
            .with(eq(hashset![token_a, token_b]), eq(H160([0xc1; 20])))
            .returning(|_, _| Ok(Allowances::empty(H160([0xc1; 20]))));

        let base_tokens = BaseTokens::new(token_a, &[]);
        let pairs =
            base_tokens.relevant_pairs([TokenPair::new(token_a, token_b).unwrap()].into_iter());

        let (settlement, batch_router) = dummy_contracts();
        let liquidity_provider = BalancerV3Liquidity {
            settlement,
            batch_router,
            pool_fetcher: Arc::new(pool_fetcher),
            allowance_manager: Box::new(allowance_manager),
        };
        let (
            _stable_orders,
            _stable_surge_orders,
            _weighted_orders,
            _gyro_e_orders,
            _gyro_2clp_orders,
            reclamm_orders,
            _quantamm_orders,
        ) = liquidity_provider
            .get_orders(pairs, Block::Recent)
            .await
            .unwrap();

        assert_eq!(reclamm_orders.len(), 1);
        let order = &reclamm_orders[0];
        assert_eq!(order.address, H160([0x95; 20]));
        assert_eq!(order.fee, "0.003".parse().unwrap());
        assert_eq!(
            order.version,
            shared::sources::balancer_v3::pools::reclamm::Version::V2
        );
        assert_eq!(
            order.last_virtual_balances,
            vec![10u64.into(), 20u64.into()]
        );
    }

    #[test]
    fn encodes_reclamm_swaps_in_settlement() {
        let (settlement, batch_router) = dummy_contracts();
        let inner = Arc::new(Inner {
            settlement: settlement.clone(),
            batch_router: batch_router.clone(),
            allowances: Allowances::new(
                batch_router.address().into_legacy(),
                hashmap! {
                    H160([0xaa; 20]) => 0.into(),
                    H160([0xbb; 20]) => 100.into(),
                },
            ),
        });
        let handler = SettlementHandler {
            pool_id: H160([0x95; 20]),
            inner,
        };

        let mut encoder = SettlementEncoder::new(Default::default());
        SettlementHandling::<BalancerV3ReClammOrder>::encode(
            &handler,
            AmmOrderExecution {
                input_max: TokenAmount::new(H160([0xaa; 20]), 10),
                output: TokenAmount::new(H160([0xbb; 20]), 11),
                internalizable: false,
            },
            &mut encoder,
        )
        .unwrap();
        SettlementHandling::<BalancerV3ReClammOrder>::encode(
            &handler,
            AmmOrderExecution {
                input_max: TokenAmount::new(H160([0xbb; 20]), 12),
                output: TokenAmount::new(H160([0xcc; 20]), 13),
                internalizable: false,
            },
            &mut encoder,
        )
        .unwrap();

        let [_, interactions, _] = encoder
            .finish(InternalizationStrategy::SkipInternalizableInteraction)
            .interactions;
        assert_eq!(interactions.len(), 3);
    }

    #[test]
    fn encodes_swaps_in_settlement() {
        let (settlement, batch_router) = dummy_contracts();
        let inner = Arc::new(Inner {
            settlement: settlement.clone(),
            batch_router: batch_router.clone(),
            allowances: Allowances::new(
                batch_router.address().into_legacy(),
                hashmap! {
                    H160([0x70; 20]) => 0.into(),
                    H160([0x71; 20]) => 100.into(),
                },
            ),
        });
        let handler = SettlementHandler {
            pool_id: H160([0x90; 20]),
            inner,
        };

        let mut encoder = SettlementEncoder::new(Default::default());
        SettlementHandling::<BalancerV3WeightedProductOrder>::encode(
            &handler,
            AmmOrderExecution {
                input_max: TokenAmount::new(H160([0x70; 20]), 10),
                output: TokenAmount::new(H160([0x71; 20]), 11),
                internalizable: false,
            },
            &mut encoder,
        )
        .unwrap();
        SettlementHandling::<BalancerV3WeightedProductOrder>::encode(
            &handler,
            AmmOrderExecution {
                input_max: TokenAmount::new(H160([0x71; 20]), 12),
                output: TokenAmount::new(H160([0x72; 20]), 13),
                internalizable: false,
            },
            &mut encoder,
        )
        .unwrap();

        let [_, interactions, _] = encoder
            .finish(InternalizationStrategy::SkipInternalizableInteraction)
            .interactions;
        assert_eq!(
            interactions,
            [
                Approval {
                    token: H160([0x70; 20]),
                    spender: batch_router.address().into_legacy(),
                }
                .encode(),
                BalancerV3SwapGivenOutInteraction {
                    settlement: settlement.clone(),
                    batch_router: batch_router.clone(),
                    pool: H160([0x90; 20]),
                    asset_in_max: TokenAmount::new(H160([0x70; 20]), 10),
                    asset_out: TokenAmount::new(H160([0x71; 20]), 11),
                    user_data: Default::default(),
                }
                .encode(),
                BalancerV3SwapGivenOutInteraction {
                    settlement,
                    batch_router,
                    pool: H160([0x90; 20]),
                    asset_in_max: TokenAmount::new(H160([0x71; 20]), 12),
                    asset_out: TokenAmount::new(H160([0x72; 20]), 13),
                    user_data: Default::default(),
                }
                .encode(),
            ],
        );
    }
}
