use {
    super::{Fee, Id, ScalingFactor},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
    itertools::Itertools,
};

/// Liquidity data tied to a Balancer V2 Gyroscope 3-CLP pool.
///
/// Gyroscope 3-CLP (3 Constant Liquidity Pool) is an AMM that uses a
/// cubic invariant curve for improved capital efficiency with three assets.
/// The pool's shape is defined by a static parameter (root3_alpha) that is
/// immutable after pool creation. The invariant is (x + a)(y + a)(z + a) = k,
/// where x, y, z are the real reserves and 'a' is the virtual reserve added
/// to each asset to define the price range.
///
/// References:
/// - [Gyroscope 3-CLP Documentation](https://docs.gyro.finance/pools/3-clps.html)
#[derive(Clone, Debug)]
pub struct Pool {
    pub vault: eth::ContractAddress,
    pub id: Id,
    pub reserves: Reserves,
    pub fee: Fee,
    pub version: Version,
    // Gyroscope 3-CLP static parameter (immutable after pool creation)
    pub root3_alpha: FixedPoint,
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
            boundary::liquidity::balancer::v2::gyro_3clp::to_interaction(
                self, input, output, receiver,
            ),
        )
    }
}

/// Balancer Gyroscope 3-CLP pool reserves.
///
/// This is an ordered collection of exactly three tokens with their balance
/// and scaling factors.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Creates new Balancer V2 token reserves for 3-CLP, returns `Err` if the
    /// specified token reserves are invalid. For 3-CLP pools, exactly 3 unique
    /// tokens are required.
    pub fn try_new(reserves: Vec<Reserve>) -> Result<Self, InvalidReserves> {
        if reserves.len() != 3 {
            return Err(InvalidReserves);
        }

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
#[error("invalid Balancer V2 token reserves for 3-CLP pool")]
pub struct InvalidReserves;

/// A Balancer V2 token reserve.
#[derive(Clone, Copy, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: ScalingFactor,
    pub rate: eth::U256,
}

/// Fixed point number used for Gyroscope 3-CLP parameters.
///
/// Gyroscope 3-CLP parameters use fixed point arithmetic for precise
/// mathematical calculations. This is a wrapper around the underlying U256
/// type for the root3_alpha parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedPoint(pub ethcontract::U256);

impl FixedPoint {
    /// Creates a new fixed point from raw wei value.
    pub fn from_raw(value: ethcontract::U256) -> Self {
        Self(value)
    }

    /// Returns the raw wei value.
    pub fn as_raw(&self) -> ethcontract::U256 {
        self.0
    }
}

impl Default for FixedPoint {
    fn default() -> Self {
        Self(ethcontract::U256::zero())
    }
}

/// The Gyroscope 3-CLP pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of Gyroscope 3-CLP pools.
    V1,
}

impl Default for Version {
    fn default() -> Self {
        Self::V1
    }
}
