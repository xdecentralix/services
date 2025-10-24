use {
    crate::domain::{eth, liquidity},
    itertools::Itertools as _,
};

#[derive(Clone, Debug)]
pub struct Pool {
    pub reserves: Reserves,
    pub fee: eth::Rational,
    pub version: Version,
    // QuantAMM-specific parameters for weight interpolation
    pub max_trade_size_ratio: eth::Rational,
    pub first_four_weights_and_multipliers: Vec<eth::SignedRational>,
    pub second_four_weights_and_multipliers: Vec<eth::SignedRational>,
    pub last_update_time: u64,
    pub last_interop_time: u64,
    pub current_timestamp: u64,
}

/// A representation of QuantAMM pool reserves.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Returns a new reserve instance for specified reserve entries. Returns
    /// `None` if it encounters duplicate entries for a token.
    pub fn new(mut reserves: Vec<Reserve>) -> Option<Self> {
        // Sort the reserves by their token address to ensure consistent ordering
        // (following the same pattern as GyroE and Stable pools)
        reserves.sort_unstable_by_key(|reserve| reserve.asset.token);

        let has_duplicates = reserves
            .iter()
            .tuple_windows()
            .any(|(a, b)| a.asset.token == b.asset.token);
        if has_duplicates {
            return None;
        }

        Some(Self(reserves))
    }

    /// Returns an iterator over the token reserves.
    pub fn iter(&self) -> impl Iterator<Item = Reserve> + '_ {
        self.0.iter().cloned()
    }

    /// Returns an iterator over the tokens pairs handled by the pool reserves.
    pub fn token_pairs(&self) -> impl Iterator<Item = liquidity::TokenPair> + '_ {
        self.0
            .iter()
            .tuple_combinations()
            .map(|(a, b)| liquidity::TokenPair::new(a.asset.token, b.asset.token).expect("a != b"))
    }
}

/// A QuantAMM pool token reserve.
#[derive(Clone, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: liquidity::ScalingFactor,
    pub rate: eth::Rational,
}

/// The QuantAMM pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of QuantAMM pools.
    V1,
}
