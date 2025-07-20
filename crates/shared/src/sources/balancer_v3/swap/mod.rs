//! Balancer V3 swap module containing mathematical utilities and swap logic.

use {
    crate::{
        baseline_solver::BaselineSolvable,
        conversions::U256Ext,
        sources::balancer_v3::pool_fetching::{
            TokenState,
            WeightedPool,
            WeightedTokenState,
            WeightedPoolVersion,
        },
    },
    error::Error,
    ethcontract::{H160, U256},
    fixed_point::Bfp,
    std::collections::BTreeMap,
};

mod error;
pub mod fixed_point;
mod math;
mod weighted_math;

const WEIGHTED_SWAP_GAS_COST: usize = 100_000;

fn add_swap_fee_amount(amount: U256, swap_fee: Bfp) -> Result<U256, Error> {
    // https://github.com/balancer-labs/balancer-v2-monorepo/blob/6c9e24e22d0c46cca6dd15861d3d33da61a60b98/pkg/core/contracts/pools/BasePool.sol#L454-L457
    let amount_with_fees = Bfp::from_wei(amount).div_up(swap_fee.complement())?;
    Ok(amount_with_fees.as_uint256())
}

fn subtract_swap_fee_amount(amount: U256, swap_fee: Bfp) -> Result<U256, Error> {
    // https://github.com/balancer-labs/balancer-v2-monorepo/blob/6c9e24e22d0c46cca6dd15861d3d33da61a60b98/pkg/core/contracts/pools/BasePool.sol#L462-L466
    let amount = Bfp::from_wei(amount);
    let fee_amount = amount.mul_up(swap_fee)?;
    let amount_without_fees = amount.sub(fee_amount)?;
    Ok(amount_without_fees.as_uint256())
}

impl TokenState {
    /// Converts the stored balance into its internal representation as a
    /// Balancer fixed point number.
    fn upscaled_balance(&self) -> Result<Bfp, Error> {
        self.upscale(self.balance)
    }

    /// Scales the input token amount to the value that is used by the Balancer
    /// contract to execute math operations.
    fn upscale(&self, amount: U256) -> Result<Bfp, Error> {
        Bfp::from_wei(amount).mul_down(self.scaling_factor)
    }

    /// Returns the token amount corresponding to the internal Balancer
    /// representation for the same amount.
    /// Based on contract code here:
    /// https://github.com/balancer-labs/balancer-v2-monorepo/blob/c18ff2686c61a8cbad72cdcfc65e9b11476fdbc3/pkg/pool-utils/contracts/BasePool.sol#L560-L562
    fn downscale_up(&self, amount: Bfp) -> Result<U256, Error> {
        Ok(amount.div_up(self.scaling_factor)?.as_uint256())
    }

    /// Similar to downscale up above, but rounded down, this is just checked
    /// div. https://github.com/balancer-labs/balancer-v2-monorepo/blob/c18ff2686c61a8cbad72cdcfc65e9b11476fdbc3/pkg/pool-utils/contracts/BasePool.sol#L542-L544
    fn downscale_down(&self, amount: Bfp) -> Result<U256, Error> {
        Ok(amount.div_down(self.scaling_factor)?.as_uint256())
    }
}

/// Weighted pool data as a reference used for computing input and output
/// amounts.
#[derive(Debug)]
pub struct WeightedPoolRef<'a> {
    pub reserves: &'a BTreeMap<H160, WeightedTokenState>,
    pub swap_fee: Bfp,
    pub version: WeightedPoolVersion,
}

impl WeightedPoolRef<'_> {
    fn get_amount_out_inner(
        &self,
        out_token: H160,
        in_amount: U256,
        in_token: H160,
    ) -> Option<U256> {
        // Note that the output of this function does not depend on the pool
        // specialization. All contract branches compute this amount with:
        // https://github.com/balancer-labs/balancer-v2-monorepo/blob/6c9e24e22d0c46cca6dd15861d3d33da61a60b98/pkg/core/contracts/pools/BaseMinimalSwapInfoPool.sol#L62-L75
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        let in_amount_minus_fees = subtract_swap_fee_amount(in_amount, self.swap_fee).ok()?;

        let out_amount = weighted_math::calc_out_given_in(
            in_reserves.common.upscaled_balance().ok()?,
            in_reserves.weight,
            out_reserves.common.upscaled_balance().ok()?,
            out_reserves.weight,
            in_reserves.common.upscale(in_amount_minus_fees).ok()?,
        )
        .ok()?;
        out_reserves.common.downscale_down(out_amount).ok()
    }
}

impl BaselineSolvable for WeightedPoolRef<'_> {
    async fn get_amount_out(
        &self,
        out_token: H160,
        (in_amount, in_token): (U256, H160),
    ) -> Option<U256> {
        self.get_amount_out_inner(out_token, in_amount, in_token)
    }

    async fn get_amount_in(
        &self,
        in_token: H160,
        (out_amount, out_token): (U256, H160),
    ) -> Option<U256> {
        // Note that the output of this function does not depend on the pool
        // specialization. All contract branches compute this amount with:
        // https://github.com/balancer-labs/balancer-v2-monorepo/blob/6c9e24e22d0c46cca6dd15861d3d33da61a60b98/pkg/core/contracts/pools/BaseMinimalSwapInfoPool.sol#L75-L88
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        let in_amount = weighted_math::calc_in_given_out(
            in_reserves.common.upscaled_balance().ok()?,
            in_reserves.weight,
            out_reserves.common.upscaled_balance().ok()?,
            out_reserves.weight,
            out_reserves.common.upscale(out_amount).ok()?,
        )
        .ok()?;
        let amount_in_before_fee = in_reserves.common.downscale_up(in_amount).ok()?;
        let in_amount = add_swap_fee_amount(amount_in_before_fee, self.swap_fee).ok()?;

        converge_in_amount(in_amount, out_amount, |x| {
            self.get_amount_out_inner(out_token, x, in_token)
        })
    }

    async fn gas_cost(&self) -> usize {
        WEIGHTED_SWAP_GAS_COST
    }
}

/// Balancer V3 pools are "unstable", where if you compute an input amount large
/// enough to buy X tokens, selling the computed amount over the same pool in
/// the exact same state will yield X-𝛿 tokens. To work around this, for each
/// hop, we try to converge to some sell amount >= the required buy amount.
fn converge_in_amount(
    in_amount: U256,
    exact_out_amount: U256,
    get_amount_out: impl Fn(U256) -> Option<U256>,
) -> Option<U256> {
    let out_amount = get_amount_out(in_amount)?;
    if out_amount >= exact_out_amount {
        return Some(in_amount);
    }

    // If the computed output amount is not enough; we bump the sell amount a
    // bit. We start by approximating the out amount deficit to in tokens at the
    // trading price and multiply the amount to bump by 10 for each iteration.
    let mut bump = (exact_out_amount - out_amount)
        .checked_mul(in_amount)?
        .ceil_div(&out_amount.max(U256::one()))
        .max(U256::one());

    for _ in 0..6 {
        let bumped_in_amount = in_amount.checked_add(bump)?;
        let out_amount = get_amount_out(bumped_in_amount)?;
        if out_amount >= exact_out_amount {
            return Some(bumped_in_amount);
        }

        bump *= 10;
    }

    None
}

impl WeightedPool {
    fn as_pool_ref(&self) -> WeightedPoolRef {
        WeightedPoolRef {
            reserves: &self.reserves,
            swap_fee: self.common.swap_fee,
            version: self.version,
        }
    }
}

impl BaselineSolvable for WeightedPool {
    async fn get_amount_out(&self, out_token: H160, input: (U256, H160)) -> Option<U256> {
        self.as_pool_ref().get_amount_out(out_token, input).await
    }

    async fn get_amount_in(&self, in_token: H160, output: (U256, H160)) -> Option<U256> {
        self.as_pool_ref().get_amount_in(in_token, output).await
    }

    async fn gas_cost(&self) -> usize {
        self.as_pool_ref().gas_cost().await
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sources::balancer_v3::pool_fetching::CommonPoolState,
    };

    fn create_weighted_pool_with(
        tokens: Vec<H160>,
        balances: Vec<U256>,
        weights: Vec<Bfp>,
        scaling_factors: Vec<Bfp>,
        swap_fee: U256,
    ) -> WeightedPool {
        let mut reserves = BTreeMap::new();
        for i in 0..tokens.len() {
            let (token, balance, weight, scaling_factor) =
                (tokens[i], balances[i], weights[i], scaling_factors[i]);
            reserves.insert(
                token,
                WeightedTokenState {
                    common: TokenState {
                        balance,
                        scaling_factor,
                    },
                    weight,
                },
            );
        }
        WeightedPool {
            common: CommonPoolState {
                id: Default::default(),
                address: H160::zero(),
                swap_fee: Bfp::from_wei(swap_fee),
                paused: true,
            },
            reserves,
            version: Default::default(),
        }
    }

    #[test]
    fn downscale() {
        let token_state = TokenState {
            balance: Default::default(),
            scaling_factor: Bfp::exp10(12),
        };
        let input = Bfp::from_wei(900_546_079_866_630_330_575_i128.into());
        assert_eq!(
            token_state.downscale_up(input).unwrap(),
            U256::from(900_546_080_u128)
        );
        assert_eq!(
            token_state.downscale_down(input).unwrap(),
            U256::from(900_546_079_u128)
        );
    }

    #[tokio::test]
    async fn weighted_get_amount_out() {
        // Values obtained from this transaction:
        // https://dashboard.tenderly.co/tx/main/0xa9f571c9bfd4289bd4bd270465d73e1b7e010622ed089d54d81ec63a0365ec22/debugger
        let crv = H160::repeat_byte(21);
        let sdvecrv_dao = H160::repeat_byte(42);
        let b = create_weighted_pool_with(
            vec![crv, sdvecrv_dao],
            vec![
                1_850_304_144_768_426_873_445_489_i128.into(),
                95_671_347_892_391_047_965_654_i128.into(),
            ],
            vec![bfp_v3!("0.9"), bfp_v3!("0.1")],
            vec![Bfp::exp10(0), Bfp::exp10(0)],
            2_000_000_000_000_000_i128.into(),
        );

        assert_eq!(
            b.get_amount_out(crv, (227_937_106_828_652_254_870_i128.into(), sdvecrv_dao))
                .await
                .unwrap(),
            488_192_591_864_344_551_330_i128.into()
        );
    }

    #[tokio::test]
    async fn weighted_get_amount_in() {
        // Values obtained from this transaction:
        // https://dashboard.tenderly.co/tx/main/0xafc3dd6a636a85d9c1976dfa5aee33f78e6ee902f285c9d4cf80a0014aa2a052/debugger
        let weth = H160::repeat_byte(21);
        let tusd = H160::repeat_byte(42);
        let b = create_weighted_pool_with(
            vec![weth, tusd],
            vec![60_000_000_000_000_000_i128.into(), 250_000_000_i128.into()],
            vec![bfp_v3!("0.5"), bfp_v3!("0.5")],
            vec![Bfp::exp10(0), Bfp::exp10(12)],
            1_000_000_000_000_000_i128.into(),
        );

        assert_eq!(
            b.get_amount_in(weth, (5_000_000_i128.into(), tusd))
                .await
                .unwrap(),
            1_225_715_511_429_798_i128.into()
        );
    }
} 