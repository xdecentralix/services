//! Gyroscope 3-CLP pool liquidity.

use {
    crate::domain::{eth, liquidity},
    itertools::Itertools as _,
};

/// The state of a Gyroscope 3-CLP (3 Constant Liquidity Pool).
#[derive(Clone, Debug)]
pub struct Pool {
    pub reserves: Reserves,
    pub fee: eth::Rational,
    pub version: Version,
    // Gyroscope 3-CLP static parameter (immutable after pool creation)
    pub root3_alpha: eth::Rational,
}

/// A representation of Gyroscope 3-CLP pool reserves.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Returns a new reserve instance for specified reserve entries. Returns
    /// `None` if it encounters duplicate entries for a token or if not exactly
    /// 3 tokens.
    pub fn new(mut reserves: Vec<Reserve>) -> Option<Self> {
        // 3-CLP pools must have exactly 3 tokens
        if reserves.len() != 3 {
            return None;
        }

        // Note that we sort the reserves by their token address. This is
        // because BalancerV2 3-CLP pools store their tokens in sorting order
        // - meaning that `token0` is the token address with the lowest sort
        // order. This ensures that this iterator returns the token reserves in
        // the correct order.
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

/// A Gyro 3-CLP pool token reserve.
#[derive(Clone, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: liquidity::ScalingFactor,
}

/// The Gyroscope 3-CLP pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of Gyroscope 3-CLP pools.
    V1,
}
