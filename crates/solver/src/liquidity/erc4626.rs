use {
    super::{Liquidity, Settleable, SettlementHandling},
    crate::{
        interactions::{
            Erc20ApproveInteraction,
            erc4626::{MintExactSharesInteraction, WithdrawExactAssetsInteraction},
        },
        liquidity_collector::LiquidityCollecting,
        settlement::SettlementEncoder,
    },
    anyhow::Result,
    contracts::{ERC20, IERC4626, alloy::GPv2Settlement},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    model::TokenPair,
    primitive_types::U256,
    shared::{
        ethrpc::Web3,
        recent_block_cache::Block,
        sources::erc4626::{build_edges, registry::Erc4626Registry},
    },
    std::{collections::HashSet, sync::Arc},
};

#[cfg_attr(test, derive(derivative::Derivative))]
#[cfg_attr(test, derivative(PartialEq))]
#[derive(Clone, Debug)]
pub struct Erc4626WrapOrder {
    #[cfg_attr(test, derivative(PartialEq = "ignore"))]
    pub vault: IERC4626,
    #[cfg_attr(test, derivative(PartialEq = "ignore"))]
    pub underlying: ERC20,
    pub shares_out: U256,
    pub assets_in_max: U256,
    #[cfg_attr(test, derivative(PartialEq = "ignore"))]
    pub settlement: GPv2Settlement::Instance,
}

#[cfg_attr(test, derive(derivative::Derivative))]
#[cfg_attr(test, derivative(PartialEq))]
#[derive(Clone, Debug)]
pub struct Erc4626UnwrapOrder {
    #[cfg_attr(test, derivative(PartialEq = "ignore"))]
    pub vault: IERC4626,
    pub assets_out: U256,
    #[cfg_attr(test, derivative(PartialEq = "ignore"))]
    pub settlement: GPv2Settlement::Instance,
}

#[cfg_attr(test, derive(derivative::Derivative))]
#[cfg_attr(test, derivative(PartialEq))]
#[derive(Clone, Debug)]
pub struct Erc4626Order {
    pub tokens: TokenPair,
    pub wrap: Option<Erc4626WrapOrder>,
    pub unwrap: Option<Erc4626UnwrapOrder>,
}

impl Settleable for Erc4626WrapOrder {
    type Execution = Self;

    fn settlement_handling(&self) -> &dyn SettlementHandling<Self> {
        self
    }
}

impl Settleable for Erc4626UnwrapOrder {
    type Execution = Self;

    fn settlement_handling(&self) -> &dyn SettlementHandling<Self> {
        self
    }
}

#[async_trait::async_trait]
impl LiquidityCollecting for Erc4626LiquiditySource {
    async fn get_liquidity(
        &self,
        pairs: HashSet<TokenPair>,
        _at_block: Block,
    ) -> Result<Vec<Liquidity>> {
        let edges = build_edges(&self.web3, &self.registry).await;
        let mut out = Vec::new();
        for pair in pairs {
            if let Some(edges_for_pair) = edges.get(&pair) {
                for edge in edges_for_pair {
                    // Build wrap or unwrap order shell; exact amounts will be computed by route
                    // realization.
                    if pair.get() == (edge.asset, edge.vault) {
                        out.push(Liquidity::Erc4626(Box::new(Erc4626Order {
                            tokens: pair,
                            wrap: Some(Erc4626WrapOrder {
                                vault: contracts::IERC4626::at(&self.web3, edge.vault),
                                underlying: contracts::ERC20::at(&self.web3, edge.asset),
                                shares_out: U256::zero(),
                                assets_in_max: U256::zero(),
                                settlement: self.settlement.clone(),
                            }),
                            unwrap: None,
                        })));
                    } else if pair.get() == (edge.vault, edge.asset) {
                        out.push(Liquidity::Erc4626(Box::new(Erc4626Order {
                            tokens: pair,
                            wrap: None,
                            unwrap: Some(Erc4626UnwrapOrder {
                                vault: contracts::IERC4626::at(&self.web3, edge.vault),
                                assets_out: U256::zero(),
                                settlement: self.settlement.clone(),
                            }),
                        })));
                    }
                }
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Debug)]
pub struct Erc4626LiquiditySource {
    pub web3: Web3,
    pub settlement: GPv2Settlement::Instance,
    pub registry: Erc4626Registry,
}

impl SettlementHandling<Erc4626WrapOrder> for Erc4626WrapOrder {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: Self, encoder: &mut SettlementEncoder) -> Result<()> {
        // bounded approve underlying -> vault for assets_in_max
        let approve = Erc20ApproveInteraction {
            token: execution.underlying.address().into_alloy(),
            spender: execution.vault.address().into_alloy(),
            amount: execution.assets_in_max.into_alloy(),
        };
        encoder.append_to_execution_plan(Arc::new(approve));

        // mint shares_out to settlement
        let interaction = MintExactSharesInteraction {
            vault: execution.vault.clone(),
            shares_out: execution.shares_out,
            receiver: execution.settlement.address().into_legacy(),
        };
        encoder.append_to_execution_plan(Arc::new(interaction));
        Ok(())
    }
}

impl SettlementHandling<Erc4626UnwrapOrder> for Erc4626UnwrapOrder {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn encode(&self, execution: Self, encoder: &mut SettlementEncoder) -> Result<()> {
        let settlement = execution.settlement.address().into_legacy();
        let interaction = WithdrawExactAssetsInteraction {
            vault: execution.vault.clone(),
            assets_out: execution.assets_out,
            receiver: settlement,
            owner: settlement,
        };
        encoder.append_to_execution_plan(Arc::new(interaction));
        Ok(())
    }
}
