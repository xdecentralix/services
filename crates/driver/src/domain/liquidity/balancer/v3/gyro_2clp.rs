use {
    super::{Fee, Id, ScalingFactor},
    crate::{
        boundary,
        domain::{eth, liquidity},
    },
    ethcontract::I256,
    itertools::Itertools,
};

/// Liquidity data tied to a Balancer V3 Gyroscope 2-CLP pool.
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
    pub batch_router: eth::ContractAddress,
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
    /// consumes too much gas for a single transaction.
    ///
    /// The swap is encoded as a `swapExactOut` call to the Balancer V3 Batch
    /// Router.
    pub fn swap(
        &self,
        input: &liquidity::MaxInput,
        output: &liquidity::ExactOutput,
        receiver: &eth::Address,
    ) -> Result<eth::Interaction, boundary::Error> {
        Ok(
            crate::boundary::liquidity::balancer::v3::gyro_2clp::to_interaction(
                self, input, output, receiver,
            ),
        )
    }
}

/// Token reserves for a Balancer V3 Gyroscope 2-CLP pool.
///
/// This is stored as a sorted collection of reserves to ensure deterministic
/// ordering for consistent pool interactions.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Creates new token reserves. Returns `Err` if there are any tokens with
    /// duplicate addresses.
    pub fn try_new(mut reserves: Vec<Reserve>) -> Result<Self, InvalidReserves> {
        reserves.sort_by_key(|reserve| reserve.asset.token);

        // Check for duplicate token addresses
        let duplicate = reserves
            .iter()
            .tuple_windows()
            .any(|(a, b)| a.asset.token == b.asset.token);
        if duplicate {
            return Err(InvalidReserves);
        }

        Ok(Self(reserves))
    }

    /// Returns the number of token reserves in the pool.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the pool has no token reserves.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns an iterator over the token reserves.
    pub fn iter(&self) -> impl Iterator<Item = &Reserve> + ExactSizeIterator + Clone + '_ {
        self.0.iter()
    }

    /// Returns an iterator over the token reserves by value.
    pub fn iter_copied(&self) -> impl Iterator<Item = Reserve> + ExactSizeIterator + Clone + '_ {
        self.0.iter().copied()
    }

    /// Returns an iterator over the token addresses in the pool.
    pub fn tokens(&self) -> impl Iterator<Item = eth::TokenAddress> + '_ {
        self.iter().map(|r| r.asset.token)
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

/// Balancer Gyroscope 2-CLP pool reserve for a single token.
#[derive(Clone, Copy, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: ScalingFactor,
}

/// Signed fixed point number used for Gyroscope 2-CLP parameters.
///
/// Gyroscope 2-CLP parameters use signed fixed point arithmetic for precise
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
