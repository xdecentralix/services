use {
    super::{Fee, Id, ScalingFactor},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
    itertools::Itertools,
};

/// Liquidity data tied to a Balancer V3 QuantAMM pool.
///
/// QuantAMM pools are time-weighted weighted pools that interpolate weights
/// over time using multipliers. They use the same mathematical foundation
/// as weighted pools but with dynamic weight calculation.
#[derive(Clone, Debug)]
pub struct Pool {
    pub batch_router: eth::ContractAddress,
    pub id: Id,
    pub reserves: Reserves,
    pub fee: Fee,
    pub version: Version,
    // QuantAMM-specific parameters for weight interpolation
    pub max_trade_size_ratio: ScalingFactor,
    pub first_four_weights_and_multipliers: Vec<ethcontract::I256>,
    pub second_four_weights_and_multipliers: Vec<ethcontract::I256>,
    pub last_update_time: u64,
    pub last_interop_time: u64,
    pub current_timestamp: u64,
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
        if !self.reserves.has_tokens(&input.0.token, &output.0.token) {
            return Err(liquidity::InvalidSwap);
        }

        Ok(boundary::liquidity::balancer::v3::quantamm::to_interaction(
            self, input, output, receiver,
        ))
    }
}

/// Balancer V3 QuantAMM pool reserves.
///
/// This is an ordered collection of tokens with their balance and scaling
/// factors. QuantAMM pools use the same reserve structure as regular pools
/// since weights are calculated dynamically.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    pub fn try_new(reserves: Vec<Reserve>) -> Result<Self, InvalidReserves> {
        if !reserves.iter().map(|r| r.asset.token).all_unique() {
            return Err(InvalidReserves);
        }
        Ok(Self(reserves))
    }

    fn has_tokens(&self, a: &eth::TokenAddress, b: &eth::TokenAddress) -> bool {
        self.tokens().contains(a) && self.tokens().contains(b)
    }

    pub fn tokens(&self) -> impl Iterator<Item = eth::TokenAddress> + '_ {
        self.iter().map(|r| r.asset.token)
    }

    pub fn iter(&self) -> impl Iterator<Item = Reserve> + '_ {
        self.0.iter().copied()
    }
}

impl IntoIterator for Reserves {
    type IntoIter = <Vec<Reserve> as IntoIterator>::IntoIter;
    type Item = Reserve;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid Balancer V3 token reserves; duplicate token address")]
pub struct InvalidReserves;

/// QuantAMM pool reserve for a single token.
#[derive(Clone, Copy, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: ScalingFactor,
    pub rate: eth::U256,
}

#[derive(Clone, Copy, Debug)]
pub enum Version {
    V1,
}
