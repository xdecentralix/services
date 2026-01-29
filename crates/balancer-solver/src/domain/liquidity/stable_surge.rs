//! StableSurge pool domain model.
//!
//! StableSurge pools are stable pools with dynamic surge pricing based on pool
//! imbalance.

use {super::stable, crate::domain::eth};

/// A StableSurge pool with dynamic fee based on imbalance.
#[derive(Clone, Debug)]
pub struct Pool {
    pub reserves: stable::Reserves,
    pub amplification_parameter: eth::Rational,
    pub fee: eth::Rational,
    /// Percentage threshold above which surge fees are applied
    pub surge_threshold_percentage: eth::Rational,
    /// Maximum additional fee percentage that can be applied
    pub max_surge_fee_percentage: eth::Rational,
}
