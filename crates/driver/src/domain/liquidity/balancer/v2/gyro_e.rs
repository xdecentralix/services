use {
    super::{Fee, Id, ScalingFactor},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
    ethcontract::I256,
    itertools::Itertools,
};

/// Liquidity data tied to a Balancer V2 Gyroscope E-CLP pool.
///
/// Gyroscope E-CLP (Elliptic Constant Liquidity Pool) is an advanced AMM that uses
/// an elliptical invariant curve for improved capital efficiency and customizable
/// price curves. The pool's shape is defined by static parameters that are immutable
/// after pool creation.
///
/// References:
/// - [Gyroscope E-CLP Whitepaper](https://docs.gyro.finance/pools/e-clps.html)
#[derive(Clone, Debug)]
pub struct Pool {
    pub vault: eth::ContractAddress,
    pub id: Id,
    pub reserves: Reserves,
    pub fee: Fee,
    pub version: Version,
    // Gyroscope E-CLP static parameters (immutable after pool creation)
    pub params_alpha: SignedFixedPoint,
    pub params_beta: SignedFixedPoint,
    pub params_c: SignedFixedPoint,
    pub params_s: SignedFixedPoint,
    pub params_lambda: SignedFixedPoint,
    pub tau_alpha_x: SignedFixedPoint,
    pub tau_alpha_y: SignedFixedPoint,
    pub tau_beta_x: SignedFixedPoint,
    pub tau_beta_y: SignedFixedPoint,
    pub u: SignedFixedPoint,
    pub v: SignedFixedPoint,
    pub w: SignedFixedPoint,
    pub z: SignedFixedPoint,
    pub d_sq: SignedFixedPoint,
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

        Ok(boundary::liquidity::balancer::v2::gyro_e::to_interaction(
            self, input, output, receiver,
        ))
    }
}

/// Balancer Gyroscope E-CLP pool reserves.
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
#[error("invalid Balancer V2 token reserves; duplicate token address")]
pub struct InvalidReserves;

/// Balancer Gyroscope E-CLP pool reserve for a single token.
#[derive(Clone, Copy, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: ScalingFactor,
}

/// Signed fixed point number used for Gyroscope E-CLP parameters.
///
/// Gyroscope E-CLP parameters use signed fixed point arithmetic for precise
/// mathematical calculations. This is a wrapper around the underlying SBfp type
/// from the shared crate.
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

/// The Gyroscope E-CLP pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of Gyroscope E-CLP pools.
    V1,
}

impl Default for Version {
    fn default() -> Self {
        Self::V1
    }
}