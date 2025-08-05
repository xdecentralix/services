use {
    crate::domain::{eth, liquidity},
    itertools::Itertools as _,
};

/// The state of a Gyroscope E-CLP (Elliptic Constant Liquidity Pool).
#[derive(Clone, Debug)]
pub struct Pool {
    pub reserves: Reserves,
    pub fee: eth::Rational,
    pub version: Version,
    // Gyroscope E-CLP static parameters (immutable after pool creation)
    // These can be negative, so we use SignedRational
    pub params_alpha: eth::SignedRational,
    pub params_beta: eth::SignedRational,
    pub params_c: eth::SignedRational,
    pub params_s: eth::SignedRational,
    pub params_lambda: eth::SignedRational,
    pub tau_alpha_x: eth::SignedRational,
    pub tau_alpha_y: eth::SignedRational,
    pub tau_beta_x: eth::SignedRational,
    pub tau_beta_y: eth::SignedRational,
    pub u: eth::SignedRational,
    pub v: eth::SignedRational,
    pub w: eth::SignedRational,
    pub z: eth::SignedRational,
    pub d_sq: eth::SignedRational,
}

/// A representation of Gyroscope E-CLP pool reserves.
#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    /// Returns a new reserve instance for specified reserve entries. Returns
    /// `None` if it encounters duplicate entries for a token.
    pub fn new(mut reserves: Vec<Reserve>) -> Option<Self> {
        // Note that we sort the reserves by their token address. This is
        // because BalancerV2 E-CLP pools store their tokens in sorting order
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

/// A Gyro E-CLP pool token reserve.
#[derive(Clone, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: liquidity::ScalingFactor,
}

/// The Gyroscope E-CLP pool version.
#[derive(Clone, Copy, Debug)]
pub enum Version {
    /// Version 1 of Gyroscope E-CLP pools.
    V1,
}