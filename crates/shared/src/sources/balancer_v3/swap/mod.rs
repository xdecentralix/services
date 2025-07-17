//! Balancer V3 swap module containing mathematical utilities and swap logic.

use {
    crate::{
        baseline_solver::BaselineSolvable,
        conversions::U256Ext,
        sources::balancer_v3::pool_fetching::{
            TokenState,
            WeightedPool,
            WeightedTokenState,
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

fn converge_in_amount(
    in_amount: U256,
    exact_out_amount: U256,
    get_amount_out: impl Fn(U256) -> Option<U256>,
) -> Option<U256> {
    // Binary search to find the exact input amount that produces the desired output
    let mut low = U256::zero();
    let mut high = in_amount;
    let mut best_in_amount = in_amount;
    let mut best_out_amount = get_amount_out(in_amount)?;

    // If we're already close enough, return the current amount
    if best_out_amount >= exact_out_amount {
        return Some(in_amount);
    }

    // Binary search with a maximum of 256 iterations to prevent infinite loops
    for _ in 0..256 {
        let mid = (low + high) / U256::from(2);
        if mid == low || mid == high {
            break;
        }

        let out_amount = get_amount_out(mid)?;
        if out_amount >= exact_out_amount {
            high = mid;
            best_in_amount = mid;
            best_out_amount = out_amount;
        } else {
            low = mid;
        }
    }

    Some(best_in_amount)
}

impl WeightedPool {
    fn as_pool_ref(&self) -> WeightedPoolRef {
        WeightedPoolRef {
            reserves: &self.reserves,
            swap_fee: self.swap_fee,
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
    use super::*;
    use crate::sources::balancer_v3::pool_fetching::common::PoolInfo;

    fn create_weighted_pool_with(
        tokens: Vec<H160>,
        balances: Vec<U256>,
        weights: Vec<Bfp>,
        scaling_factors: Vec<Bfp>,
        swap_fee: U256,
    ) -> WeightedPool {
        assert_eq!(tokens.len(), balances.len());
        assert_eq!(tokens.len(), weights.len());
        assert_eq!(tokens.len(), scaling_factors.len());

        let mut reserves = BTreeMap::new();
        for (i, token) in tokens.iter().enumerate() {
            let common = TokenState {
                balance: balances[i],
                scaling_factor: scaling_factors[i],
            };
            let weighted_token_state = WeightedTokenState {
                common,
                weight: weights[i],
            };
            reserves.insert(*token, weighted_token_state);
        }

        WeightedPool {
            common: PoolInfo {
                id: H160([1; 20]),
                address: H160([1; 20]),
                tokens,
                scaling_factors,
                block_created: 0,
            },
            reserves,
            swap_fee: Bfp::from_wei(swap_fee),
        }
    }

    #[test]
    fn downscale() {
        let token_state = TokenState {
            balance: U256::from(1000),
            scaling_factor: Bfp::from_wei(U256::from(1_000_000_000_000_000_000u128)),
        };

        let upscaled = token_state.upscaled_balance().unwrap();
        let downscaled_up = token_state.downscale_up(upscaled).unwrap();
        let downscaled_down = token_state.downscale_down(upscaled).unwrap();

        assert_eq!(downscaled_up, U256::from(1000));
        assert_eq!(downscaled_down, U256::from(1000));
    }

    #[tokio::test]
    async fn weighted_get_amount_out() {
        let tokens = vec![H160([1; 20]), H160([2; 20])];
        let balances = vec![U256::from(1_000_000), U256::from(1_000_000)];
        let weights = vec![Bfp::from_wei(U256::from(500_000_000_000_000_000u128)), Bfp::from_wei(U256::from(500_000_000_000_000_000u128))];
        let scaling_factors = vec![Bfp::from_wei(U256::from(1_000_000_000_000_000_000u128)), Bfp::from_wei(U256::from(1_000_000_000_000_000_000u128))];
        let swap_fee = U256::from(3_000_000_000_000_000u128); // 0.3%

        let pool = create_weighted_pool_with(tokens, balances, weights, scaling_factors, swap_fee);

        let amount_out = pool
            .get_amount_out(H160([2; 20]), (U256::from(100_000), H160([1; 20])))
            .await
            .unwrap();

        // Should be less than input due to fees and slippage
        assert!(amount_out < U256::from(100_000));
        assert!(amount_out > U256::zero());
    }

    #[tokio::test]
    async fn weighted_get_amount_in() {
        let tokens = vec![H160([1; 20]), H160([2; 20])];
        let balances = vec![U256::from(1_000_000), U256::from(1_000_000)];
        let weights = vec![Bfp::from_wei(U256::from(500_000_000_000_000_000u128)), Bfp::from_wei(U256::from(500_000_000_000_000_000u128))];
        let scaling_factors = vec![Bfp::from_wei(U256::from(1_000_000_000_000_000_000u128)), Bfp::from_wei(U256::from(1_000_000_000_000_000_000u128))];
        let swap_fee = U256::from(3_000_000_000_000_000u128); // 0.3%

        let pool = create_weighted_pool_with(tokens, balances, weights, scaling_factors, swap_fee);

        let amount_in = pool
            .get_amount_in(H160([1; 20]), (U256::from(100_000), H160([2; 20])))
            .await
            .unwrap();

        // Should be more than output due to fees and slippage
        assert!(amount_in > U256::from(100_000));
    }
} 