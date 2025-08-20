use crate::{
    domain::{self, eth, liquidity},
    util::Bytes,
};

/// Interaction with a smart contract which is needed to execute this solution
/// on the blockchain.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Interaction {
    Custom(Custom),
    Liquidity(Liquidity),
}

impl Interaction {
    /// Should the interaction be internalized?
    pub fn internalize(&self) -> bool {
        match self {
            Interaction::Custom(custom) => custom.internalize,
            Interaction::Liquidity(liquidity) => liquidity.internalize,
        }
    }

    /// The assets consumed by this interaction. These assets are taken from the
    /// settlement contract when the interaction executes.
    pub fn inputs(&self) -> Vec<eth::Asset> {
        match self {
            Interaction::Custom(custom) => custom.inputs.clone(),
            Interaction::Liquidity(liquidity) => vec![liquidity.input],
        }
    }

    /// Returns the ERC20 approvals required for executing this interaction
    /// onchain.
    pub fn allowances(&self) -> Vec<eth::allowance::Required> {
        match self {
            Interaction::Custom(interaction) => interaction.allowances.clone(),
            Interaction::Liquidity(interaction) => {
                let address = match &interaction.liquidity.kind {
                    liquidity::Kind::UniswapV2(pool) => pool.router.into(),
                    liquidity::Kind::UniswapV3(pool) => pool.router.into(),
                    liquidity::Kind::BalancerV2Stable(pool) => pool.vault.into(),
                    liquidity::Kind::BalancerV3Stable(pool) => pool.batch_router.into(),
                    liquidity::Kind::BalancerV2Weighted(pool) => pool.vault.into(),
                    liquidity::Kind::BalancerV3Weighted(pool) => pool.batch_router.into(),
                    liquidity::Kind::BalancerV2GyroE(pool) => pool.vault.into(),
                    liquidity::Kind::BalancerV2Gyro2CLP(pool) => pool.vault.into(),
                    liquidity::Kind::BalancerV3GyroE(pool) => pool.batch_router.into(),
                    liquidity::Kind::BalancerV3Gyro2CLP(pool) => pool.batch_router.into(),
                    liquidity::Kind::BalancerV3ReClamm(pool) => pool.batch_router.into(),
                    liquidity::Kind::BalancerV3QuantAmm(pool) => pool.batch_router.into(),
                    liquidity::Kind::Swapr(pool) => pool.base.router.into(),
                    liquidity::Kind::ZeroEx(pool) => pool.zeroex.address().into(),
                    liquidity::Kind::Erc4626(edge) => edge.tokens.1.0.into(),
                };
                match &interaction.liquidity.kind {
                    // For AMMs and 0x, keep using max approvals
                    liquidity::Kind::UniswapV2(_)
                    | liquidity::Kind::UniswapV3(_)
                    | liquidity::Kind::BalancerV2Stable(_)
                    | liquidity::Kind::BalancerV3Stable(_)
                    | liquidity::Kind::BalancerV2Weighted(_)
                    | liquidity::Kind::BalancerV3Weighted(_)
                    | liquidity::Kind::BalancerV2GyroE(_)
                    | liquidity::Kind::BalancerV2Gyro2CLP(_)
                    | liquidity::Kind::BalancerV3GyroE(_)
                    | liquidity::Kind::BalancerV3Gyro2CLP(_)
                    | liquidity::Kind::BalancerV3ReClamm(_)
                    | liquidity::Kind::BalancerV3QuantAmm(_)
                    | liquidity::Kind::Swapr(_)
                    | liquidity::Kind::ZeroEx(_) => vec![
                        eth::Allowance {
                            token: interaction.input.token,
                            spender: address,
                            amount: eth::U256::max_value(),
                        }
                        .into(),
                    ],
                    liquidity::Kind::Erc4626(edge) => {
                        // For ERC4626, only require bounded approval on wrap (asset->vault)
                        // direction. Wrap if input token equals asset and
                        // output token equals vault.
                        if interaction.input.token == edge.tokens.0
                            && interaction.output.token == edge.tokens.1
                        {
                            vec![
                                eth::Allowance {
                                    token: interaction.input.token,
                                    spender: address,
                                    amount: interaction.input.amount.into(),
                                }
                                .into(),
                            ]
                        } else {
                            // Unwrap: no approval required (owner is settlement)
                            vec![]
                        }
                    }
                }
            }
        }
    }
}

/// An arbitrary interaction with any smart contract.
#[derive(Debug, Clone)]
pub struct Custom {
    pub target: eth::ContractAddress,
    pub value: eth::Ether,
    pub call_data: Bytes<Vec<u8>>,
    pub allowances: Vec<eth::allowance::Required>,
    /// See the [`Interaction::inputs`] method.
    pub inputs: Vec<eth::Asset>,
    /// See the [`Interaction::outputs`] method.
    pub outputs: Vec<eth::Asset>,
    /// Can the interaction be executed using the liquidity of our settlement
    /// contract?
    pub internalize: bool,
}

/// An interaction with one of the smart contracts for which we index
/// liquidity.
#[derive(Debug, Clone)]
pub struct Liquidity {
    pub liquidity: domain::Liquidity,
    /// See the [`Interaction::inputs`] method.
    pub input: eth::Asset,
    /// See the [`Interaction::outputs`] method.
    pub output: eth::Asset,
    /// Can the interaction be executed using the funds which belong to our
    /// settlement contract?
    pub internalize: bool,
}
