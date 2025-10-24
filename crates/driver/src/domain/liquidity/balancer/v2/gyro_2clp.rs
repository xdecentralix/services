use {
    super::{Fee, Id, ScalingFactor},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
    ethcontract::I256,
    itertools::Itertools,
};

/// Liquidity data tied to a Balancer V2 Gyroscope 2-CLP pool.
///
/// Gyroscope 2-CLP (2 Constant Liquidity Pool) is an AMM that uses a
/// sophisticated invariant curve for improved capital efficiency. The pool's
/// shape is defined by static parameters (sqrt_alpha and sqrt_beta) that are
/// immutable after pool creation.
///
/// References:
/// - [Gyroscope 2-CLP Documentation](https://docs.gyro.finance/pools/2-clps.html)
#[derive(Clone, Debug)]
pub struct Pool {
    pub vault: eth::ContractAddress,
    pub id: Id,
    pub reserves: Reserves,
    pub fee: Fee,
    pub version: Version,
    // Gyroscope 2-CLP static parameters (immutable after pool creation)
    pub sqrt_alpha: SignedFixedPoint,
    pub sqrt_beta: SignedFixedPoint,
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

        Ok(
            boundary::liquidity::balancer::v2::gyro_2clp::to_interaction(
                self, input, output, receiver,
            ),
        )
    }
}

/// Balancer Gyroscope 2-CLP pool reserves.
///
/// This is an ordered collection of tokens with their balance and scaling
/// factors.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Creates new Balancer V2 token reserves, returns `Err` if the specified
    /// token reserves are invalid, specifically, if there are duplicate tokens.
    pub fn try_new(reserves: Vec<Reserve>) -> Result<Self, InvalidReserves> {
        if !reserves.iter().map(|r| r.asset.token).all_unique() {
            return Err(InvalidReserves);
        }

        Ok(Self(reserves))
    }

    /// Returns `true` if the reserves correspond to the specified tokens.
    fn has_tokens(&self, a: &eth::TokenAddress, b: &eth::TokenAddress) -> bool {
        self.tokens().contains(a) && self.tokens().contains(b)
    }

    /// Returns an iterator over the reserve tokens.
    pub fn tokens(&self) -> impl Iterator<Item = eth::TokenAddress> + '_ {
        self.iter().map(|r| r.asset.token)
    }

    /// Returns an iterator over the reserve assets.
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
#[error("invalid Balancer V2 token reserves")]
pub struct InvalidReserves;

/// A Balancer V2 token reserve.
#[derive(Clone, Copy, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: ScalingFactor,
    pub rate: eth::U256,
}

/// Signed fixed point number used for Gyroscope 2-CLP parameters.
///
/// Gyroscope 2-CLP parameters use signed fixed point arithmetic for precise
/// mathematical calculations. This is a wrapper around the underlying I256
/// type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedFixedPoint(pub I256);

impl SignedFixedPoint {
    /// Creates a new signed fixed point from raw wei value.
    pub fn from_raw(value: I256) -> Self {
        Self(value)
    }

    /// Returns the raw wei value.
    pub fn as_raw(&self) -> I256 {
        self.0
    }
}

impl Default for SignedFixedPoint {
    fn default() -> Self {
        Self(I256::zero())
    }
}

/// The Gyroscope 2-CLP pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of Gyroscope 2-CLP pools.
    V1,
}

impl Default for Version {
    fn default() -> Self {
        Self::V1
    }
}
