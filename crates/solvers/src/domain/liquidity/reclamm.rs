use {crate::domain::eth, itertools::Itertools as _};

#[derive(Clone, Debug)]
pub struct Pool {
    pub reserves: Reserves,
    pub fee: eth::Rational,
    // Dynamic parameters needed for ReCLAMM math
    pub last_virtual_balances: Vec<eth::Rational>,
    pub daily_price_shift_base: eth::Rational,
    pub last_timestamp: u64,
    pub centeredness_margin: eth::Rational,
    pub start_fourth_root_price_ratio: eth::Rational,
    pub end_fourth_root_price_ratio: eth::Rational,
    pub price_ratio_update_start_time: u64,
    pub price_ratio_update_end_time: u64,
}

#[derive(Clone, Debug)]
pub struct Reserves(Vec<Reserve>);

impl Reserves {
    pub fn try_new(reserves: Vec<Reserve>) -> Result<Self, ()> {
        let has_dups = reserves
            .iter()
            .map(|r| r.asset.token)
            .tuple_windows()
            .any(|(a, b)| a == b);
        if has_dups {
            return Err(());
        }
        Ok(Self(reserves))
    }

    pub fn iter(&self) -> impl Iterator<Item = Reserve> + '_ {
        self.0.iter().cloned()
    }

    pub fn token_pairs(&self) -> impl Iterator<Item = super::TokenPair> + '_ {
        self.0
            .iter()
            .map(|r| r.asset.token)
            .tuple_combinations()
            .filter_map(|(a, b)| super::TokenPair::new(a, b))
    }
}

#[derive(Clone, Debug)]
pub struct Reserve {
    pub asset: eth::Asset,
    pub scale: super::ScalingFactor,
}
