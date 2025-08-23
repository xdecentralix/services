use {
    super::{Fee, Id, stable},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
};

/// Liquidity data tied to a Balancer V3 StableSurge pool.
///
/// StableSurge pools are stable pools with a dynamic fee hook that adjusts fees
/// based on pool imbalance. The hook implements surge pricing to discourage
/// trades that increase imbalance and incentivize trades that restore balance.
/// These pools use the same core stable math as regular stable pools but with
/// dynamic fee calculation.
///
/// [^1]: <https://classic.curve.fi/whitepaper>
/// [^2]: <https://docs.balancer.fi/products/balancer-pools/stable-pools>
/// [^3]: <https://github.com/balancer/balancer-v3-monorepo/blob/main/pkg/pool-hooks/contracts/StableSurgeHook.sol>
#[derive(Clone, Debug)]
pub struct Pool {
    pub batch_router: eth::ContractAddress,
    pub id: Id,
    pub reserves: stable::Reserves,
    pub amplification_parameter: stable::AmplificationParameter,
    pub fee: Fee,
    pub version: stable::Version,
    // StableSurge hook parameters
    pub surge_threshold_percentage: SurgeThresholdPercentage,
    pub max_surge_fee_percentage: MaxSurgeFeePercentage,
}

impl Pool {
    /// Encodes a pool swap as an interaction. Returns `Err` if the swap
    /// parameters are invalid for the pool, specifically if the input and
    /// output tokens do not belong to the pool.
    pub fn swap(
        &self,
        input: &liquidity::MaxInput,
        output: &liquidity::ExactOutput,
        receiver: &eth::Address,
    ) -> Result<eth::Interaction, liquidity::InvalidSwap> {
        // Check if both tokens exist in the reserves
        if !self.reserves.tokens().any(|token| token == input.0.token)
            || !self.reserves.tokens().any(|token| token == output.0.token)
        {
            return Err(liquidity::InvalidSwap);
        }

        Ok(
            boundary::liquidity::balancer::v3::stable_surge::to_interaction(
                self, input, output, receiver,
            ),
        )
    }
}

/// StableSurge hook surge threshold percentage.
///
/// This represents the imbalance threshold above which surge fees are applied.
/// Internally, this is represented as an 18-decimal fixed point number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurgeThresholdPercentage {
    value: eth::U256,
}

impl SurgeThresholdPercentage {
    pub fn new(value: eth::U256) -> Result<Self, InvalidSurgeParameter> {
        // Validate that the percentage is in range [0, 1] (0% to 100%)
        if value > eth::U256::from(10).pow(18.into()) {
            return Err(InvalidSurgeParameter::OutOfRange);
        }

        Ok(Self { value })
    }

    pub fn value(&self) -> eth::U256 {
        self.value
    }
}

/// StableSurge hook maximum surge fee percentage.
///
/// This represents the maximum surge fee that can be applied on top of the
/// static swap fee. Internally, this is represented as an 18-decimal fixed
/// point number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaxSurgeFeePercentage {
    value: eth::U256,
}

impl MaxSurgeFeePercentage {
    pub fn new(value: eth::U256) -> Result<Self, InvalidSurgeParameter> {
        // Validate that the percentage is in range [0, 1] (0% to 100%)
        if value > eth::U256::from(10).pow(18.into()) {
            return Err(InvalidSurgeParameter::OutOfRange);
        }

        Ok(Self { value })
    }

    pub fn value(&self) -> eth::U256 {
        self.value
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidSurgeParameter {
    #[error("invalid StableSurge parameter; value out of range [0, 1e18]")]
    OutOfRange,
}

// Re-export stable pool types for convenience
pub use stable::{
    AmplificationParameter,
    InvalidAmplificationParameter,
    InvalidReserves,
    Reserve,
    Reserves,
    Version,
};
