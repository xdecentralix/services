//! Balancer V3 swap module containing mathematical utilities and swap logic.

use {
    crate::{
        baseline_solver::BaselineSolvable,
        conversions::U256Ext,
        sources::balancer_v3::pool_fetching::{
            AmplificationParameter,
            GyroEPool,
            GyroEPoolVersion,
            QuantAmmPool,
            ReClammPool,
            StablePool,
            TokenState,
            WeightedPool,
            WeightedPoolVersion,
            WeightedTokenState,
        },
    },
    error::Error,
    ethcontract::{H160, I256, U256},
    fixed_point::Bfp,
    num::BigInt,
    std::collections::BTreeMap,
};

mod error;
pub mod fixed_point;
pub mod gyro_e_math;
mod math;
pub mod quantamm_math;
pub mod reclamm_math;
pub mod signed_fixed_point;
mod stable_math;
mod weighted_math;

const WEIGHTED_SWAP_GAS_COST: usize = 100_000;
const STABLE_SWAP_GAS_COST: usize = 183_520;
const GYRO_E_SWAP_GAS_COST: usize = 100_000;
const RECLAMM_SWAP_GAS_COST: usize = 100_000;

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

// Apply scaling factor and rate with rounding down
fn to_scaled_18_apply_rate_round_down_bfp(
    amount: Bfp,
    scaling_factor: Bfp,
    rate: Bfp,
) -> Result<Bfp, Error> {
    // Apply scaling factor first, then rate, both with rounding down
    let scaled = amount.mul_down(scaling_factor)?;
    scaled.mul_down(rate)
}

// Apply scaling factor and rate with rounding up
#[allow(dead_code)]
fn to_scaled_18_apply_rate_round_up_bfp(
    amount: Bfp,
    scaling_factor: Bfp,
    rate: Bfp,
) -> Result<Bfp, Error> {
    // Apply scaling factor first, then rate, both with rounding up
    let scaled = amount.mul_up(scaling_factor)?;
    scaled.mul_up(rate)
}

// Undo scaling factor and rate with rounding down
fn to_raw_undo_rate_round_down_bfp(
    amount: Bfp,
    scaling_factor: Bfp,
    rate: Bfp,
) -> Result<Bfp, Error> {
    // Multiply scaling factor and rate first, then divide amount by the product
    let denominator = scaling_factor.mul_up(rate)?;
    amount.div_down(denominator)
}

// Undo scaling factor and rate with rounding up
fn to_raw_undo_rate_round_up_bfp(
    amount: Bfp,
    scaling_factor: Bfp,
    rate: Bfp,
) -> Result<Bfp, Error> {
    // Multiply scaling factor and rate first, then divide amount by the product
    let denominator = scaling_factor.mul_up(rate)?;
    amount.div_up(denominator)
}

// Rate rounding function from Balancer math library
#[allow(dead_code)]
fn compute_rate_round_up(rate: U256) -> U256 {
    let rounded_rate = (rate / U256::exp10(18)) * U256::exp10(18);
    if rounded_rate == rate { rate } else { rate + 1 }
}

impl TokenState {
    /// Converts the stored balance into its internal representation as a
    /// Balancer fixed point number.
    fn upscaled_balance(&self) -> Result<Bfp, Error> {
        self.upscale(self.balance)
    }

    /// Scales the input token amount to the value that is used by the Balancer
    /// contract to execute math operations, applying rate provider if present.
    fn upscale(&self, amount: U256) -> Result<Bfp, Error> {
        let amount_bfp = Bfp::from_wei(amount);

        if self.rate != U256::exp10(18) {
            let rate_bfp = Bfp::from_wei(self.rate);
            to_scaled_18_apply_rate_round_down_bfp(amount_bfp, self.scaling_factor, rate_bfp)
        } else {
            // If no rate provider, just apply scaling factor using Bfp
            amount_bfp.mul_down(self.scaling_factor)
        }
    }

    /// Returns the token amount corresponding to the internal Balancer
    /// representation for the same amount, undoing rate provider if present.
    /// Based on contract code here:
    /// https://github.com/balancer-labs/balancer-v2-monorepo/blob/c18ff2686c61a8cbad72cdcfc65e9b11476fdbc3/pkg/pool-utils/contracts/BasePool.sol#L560-L562
    fn downscale_up(&self, amount: Bfp) -> Result<U256, Error> {
        if self.rate != U256::exp10(18) {
            let rate_bfp = Bfp::from_wei(self.rate);
            let result = to_raw_undo_rate_round_up_bfp(amount, self.scaling_factor, rate_bfp)?;
            Ok(result.as_uint256())
        } else {
            // If no rate provider, just apply scaling factor using Bfp
            Ok(amount.div_up(self.scaling_factor)?.as_uint256())
        }
    }

    /// Similar to downscale up above, but rounded down, this is just checked
    /// div. https://github.com/balancer-labs/balancer-v2-monorepo/blob/c18ff2686c61a8cbad72cdcfc65e9b11476fdbc3/pkg/pool-utils/contracts/BasePool.sol#L542-L544
    fn downscale_down(&self, amount: Bfp) -> Result<U256, Error> {
        if self.rate != U256::exp10(18) {
            let rate_bfp = Bfp::from_wei(self.rate);
            let result = to_raw_undo_rate_round_down_bfp(amount, self.scaling_factor, rate_bfp)?;
            Ok(result.as_uint256())
        } else {
            // If no rate provider, just apply scaling factor using Bfp
            Ok(amount.div_down(self.scaling_factor)?.as_uint256())
        }
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

/// Stable pool data as a reference used for computing input and output amounts.
#[derive(Debug)]
pub struct StablePoolRef<'a> {
    pub address: H160,
    pub reserves: &'a BTreeMap<H160, TokenState>,
    pub swap_fee: Bfp,
    pub amplification_parameter: AmplificationParameter,
}

#[derive(Debug)]
struct BalancesWithIndices {
    token_index_in: usize,
    token_index_out: usize,
    balances: Vec<Bfp>,
}

impl<'a> StablePoolRef<'a> {
    /// This method returns an iterator over the stable pool reserves while
    /// filtering out the BPT token for the pool (i.e. the pool address). This
    /// is used because composable stable pools include their own BPT token
    /// (i.e. the ERC-20 at the pool address) in its registered tokens (i.e. the
    /// ERC-20s that can be swapped over the Balancer V2 Vault), however, this
    /// token is ignored when computing input and output amounts for regular
    /// swaps.
    ///
    /// <https://etherscan.io/address/0xf9ac7B9dF2b3454E841110CcE5550bD5AC6f875F#code#F2#L278>
    pub fn reserves_without_bpt(&self) -> impl Iterator<Item = (H160, TokenState)> + 'a + use<'a> {
        let bpt = self.address;
        self.reserves
            .iter()
            .map(|(token, state)| (*token, *state))
            .filter(move |&(token, _)| token != bpt)
    }

    fn upscale_balances_with_token_indices(
        &self,
        in_token: &H160,
        out_token: &H160,
    ) -> Result<BalancesWithIndices, Error> {
        let mut balances = vec![];
        let (mut token_index_in, mut token_index_out) = (0, 0);

        for (index, (token, balance)) in self.reserves_without_bpt().enumerate() {
            if token == *in_token {
                token_index_in = index;
            }
            if token == *out_token {
                token_index_out = index;
            }
            balances.push(balance.upscaled_balance()?)
        }
        Ok(BalancesWithIndices {
            token_index_in,
            token_index_out,
            balances,
        })
    }

    fn amplification_parameter_u256(&self) -> Option<U256> {
        self.amplification_parameter
            .with_base(*stable_math::AMP_PRECISION)
    }

    /// Comes from `_onRegularSwap(true, ...)`:
    /// https://etherscan.io/address/0xf9ac7B9dF2b3454E841110CcE5550bD5AC6f875F#code#F2#L270
    fn regular_swap_given_in(
        &self,
        out_token: H160,
        (in_amount, in_token): (U256, H160),
    ) -> Option<U256> {
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;
        let BalancesWithIndices {
            token_index_in,
            token_index_out,
            mut balances,
        } = self
            .upscale_balances_with_token_indices(&in_token, &out_token)
            .ok()?;
        let in_amount_minus_fees = subtract_swap_fee_amount(in_amount, self.swap_fee).ok()?;
        let out_amount = stable_math::calc_out_given_in(
            self.amplification_parameter_u256()?,
            balances.as_mut_slice(),
            token_index_in,
            token_index_out,
            in_reserves.upscale(in_amount_minus_fees).ok()?,
        )
        .ok()?;
        out_reserves.downscale_down(out_amount).ok()
    }

    /// Comes from `_onRegularSwap(false, ...)`:
    /// https://etherscan.io/address/0xf9ac7B9dF2b3454E841110CcE5550bD5AC6f875F#code#F2#L270
    fn regular_swap_given_out(
        &self,
        in_token: H160,
        (out_amount, out_token): (U256, H160),
    ) -> Option<U256> {
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;
        let BalancesWithIndices {
            token_index_in,
            token_index_out,
            mut balances,
        } = self
            .upscale_balances_with_token_indices(&in_token, &out_token)
            .ok()?;
        let in_amount = stable_math::calc_in_given_out(
            self.amplification_parameter_u256()?,
            balances.as_mut_slice(),
            token_index_in,
            token_index_out,
            out_reserves.upscale(out_amount).ok()?,
        )
        .ok()?;
        let amount_in_before_fee = in_reserves.downscale_up(in_amount).ok()?;
        add_swap_fee_amount(amount_in_before_fee, self.swap_fee).ok()
    }

    /// Comes from `_swapWithBpt`:
    // https://etherscan.io/address/0xf9ac7B9dF2b3454E841110CcE5550bD5AC6f875F#code#F2#L301
    fn swap_with_bpt(&self) -> Option<U256> {
        // TODO: We currently do not implement swapping with BPT for composable
        // stable pools.
        None
    }
}

impl StablePoolRef<'_> {
    fn get_amount_out_inner(
        &self,
        out_token: H160,
        in_amount: U256,
        in_token: H160,
    ) -> Option<U256> {
        if in_token == self.address || out_token == self.address {
            self.swap_with_bpt()
        } else {
            self.regular_swap_given_in(out_token, (in_amount, in_token))
        }
    }
}

impl BaselineSolvable for StablePoolRef<'_> {
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
        if in_token == self.address || out_token == self.address {
            self.swap_with_bpt()
        } else {
            let in_amount = self.regular_swap_given_out(in_token, (out_amount, out_token))?;
            converge_in_amount(in_amount, out_amount, |x| {
                self.get_amount_out_inner(out_token, x, in_token)
            })
        }
    }

    async fn gas_cost(&self) -> usize {
        STABLE_SWAP_GAS_COST
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
    fn as_pool_ref(&self) -> WeightedPoolRef<'_> {
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

impl StablePool {
    fn as_pool_ref(&self) -> StablePoolRef<'_> {
        StablePoolRef {
            address: self.common.address,
            reserves: &self.reserves,
            swap_fee: self.common.swap_fee,
            amplification_parameter: self.amplification_parameter,
        }
    }

    /// See [`StablePoolRef::reserves_without_bpt`].
    pub fn reserves_without_bpt(&self) -> impl Iterator<Item = (H160, TokenState)> + '_ {
        self.as_pool_ref().reserves_without_bpt()
    }
}

impl BaselineSolvable for StablePool {
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

#[derive(Debug)]
pub struct GyroEPoolRef<'a> {
    pub reserves: &'a BTreeMap<H160, TokenState>,
    pub swap_fee: Bfp,
    pub version: GyroEPoolVersion,
    pub params_alpha: signed_fixed_point::SBfp,
    pub params_beta: signed_fixed_point::SBfp,
    pub params_c: signed_fixed_point::SBfp,
    pub params_s: signed_fixed_point::SBfp,
    pub params_lambda: signed_fixed_point::SBfp,
    pub tau_alpha_x: signed_fixed_point::SBfp,
    pub tau_alpha_y: signed_fixed_point::SBfp,
    pub tau_beta_x: signed_fixed_point::SBfp,
    pub tau_beta_y: signed_fixed_point::SBfp,
    pub u: signed_fixed_point::SBfp,
    pub v: signed_fixed_point::SBfp,
    pub w: signed_fixed_point::SBfp,
    pub z: signed_fixed_point::SBfp,
    pub d_sq: signed_fixed_point::SBfp,
}

impl GyroEPoolRef<'_> {
    fn get_amount_out_inner(
        &self,
        out_token: H160,
        in_amount: U256,
        in_token: H160,
    ) -> Option<U256> {
        // Get token reserves
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        // Apply swap fee to input amount
        let in_amount_minus_fees = subtract_swap_fee_amount(in_amount, self.swap_fee).ok()?;

        // Determine token order (token0 vs token1)
        let token_in_is_token0 = in_token < out_token;

        // Convert reserves to the format expected by gyro_e_math
        let _balances = if token_in_is_token0 {
            vec![
                in_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
                out_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
            ]
        } else {
            vec![
                out_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
                in_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
            ]
        };

        // Convert input amount to BigInt
        let in_amount_scaled = in_reserves.upscale(in_amount_minus_fees).ok()?;
        let _amount_in_big_int = in_amount_scaled.as_uint256().to_big_int();

        // Convert SBfp parameters to gyro_e_math format and perform swap calculation
        let params = gyro_e_math::EclpParams {
            alpha: self.params_alpha.to_big_int(),
            beta: self.params_beta.to_big_int(),
            c: self.params_c.to_big_int(),
            s: self.params_s.to_big_int(),
            lambda: self.params_lambda.to_big_int(),
        };

        let derived = gyro_e_math::DerivedEclpParams {
            tau_alpha: gyro_e_math::Vector2 {
                x: self.tau_alpha_x.to_big_int(),
                y: self.tau_alpha_y.to_big_int(),
            },
            tau_beta: gyro_e_math::Vector2 {
                x: self.tau_beta_x.to_big_int(),
                y: self.tau_beta_y.to_big_int(),
            },
            u: self.u.to_big_int(),
            v: self.v.to_big_int(),
            w: self.w.to_big_int(),
            z: self.z.to_big_int(),
            d_sq: self.d_sq.to_big_int(),
        };

        // Calculate the current invariant from pool balances using gyro_e_math
        let (current_invariant, inv_err) =
            gyro_e_math::calculate_invariant_with_error(&_balances, &params, &derived).ok()?;

        // Convert to Vector2 format with error bounds (as used in tests and Python
        // reference)
        let invariant = gyro_e_math::Vector2::new(
            &current_invariant + BigInt::from(2) * &inv_err, // x: upper bound
            current_invariant,                               // y: actual invariant
        );

        // Call the gyro_e_math function
        let out_amount_big_int = gyro_e_math::calc_out_given_in(
            &_balances,
            &_amount_in_big_int,
            token_in_is_token0,
            &params,
            &derived,
            &invariant,
        )
        .ok()?;

        // Convert BigInt result back to U256 and apply downscaling
        let out_amount_sbfp = signed_fixed_point::SBfp::from_big_int(&out_amount_big_int).ok()?;
        // Convert I256 to U256 by extracting bytes (assuming positive result)
        if out_amount_sbfp.is_negative() {
            return None; // Cannot handle negative amounts in baseline solver
        }
        let mut bytes = [0u8; 32];
        out_amount_sbfp.as_i256().to_big_endian(&mut bytes);
        let out_amount_u256 = U256::from_big_endian(&bytes);
        let out_amount_bfp = Bfp::from_wei(out_amount_u256);
        let out_amount_downscaled = out_reserves.downscale_down(out_amount_bfp).ok()?;

        Some(out_amount_downscaled)
    }
}

impl BaselineSolvable for GyroEPoolRef<'_> {
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
        // Get token reserves for the reverse calculation
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        // Determine token order
        let token_in_is_token0 = in_token < out_token;

        // Convert reserves to BigInt format
        let balances = if token_in_is_token0 {
            vec![
                in_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
                out_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
            ]
        } else {
            vec![
                out_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
                in_reserves
                    .upscaled_balance()
                    .ok()?
                    .as_uint256()
                    .to_big_int(),
            ]
        };

        // Scale the output amount
        let out_amount_scaled = out_reserves.upscale(out_amount).ok()?;
        let amount_out_big_int = out_amount_scaled.as_uint256().to_big_int();

        // Convert parameters (same as get_amount_out)
        let params = gyro_e_math::EclpParams {
            alpha: self.params_alpha.to_big_int(),
            beta: self.params_beta.to_big_int(),
            c: self.params_c.to_big_int(),
            s: self.params_s.to_big_int(),
            lambda: self.params_lambda.to_big_int(),
        };

        let derived = gyro_e_math::DerivedEclpParams {
            tau_alpha: gyro_e_math::Vector2 {
                x: self.tau_alpha_x.to_big_int(),
                y: self.tau_alpha_y.to_big_int(),
            },
            tau_beta: gyro_e_math::Vector2 {
                x: self.tau_beta_x.to_big_int(),
                y: self.tau_beta_y.to_big_int(),
            },
            u: self.u.to_big_int(),
            v: self.v.to_big_int(),
            w: self.w.to_big_int(),
            z: self.z.to_big_int(),
            d_sq: self.d_sq.to_big_int(),
        };

        // Calculate the current invariant from pool balances using gyro_e_math
        let (current_invariant, inv_err) =
            gyro_e_math::calculate_invariant_with_error(&balances, &params, &derived).ok()?;

        // Convert to Vector2 format with error bounds (as used in tests and Python
        // reference)
        let invariant = gyro_e_math::Vector2::new(
            &current_invariant + BigInt::from(2) * &inv_err, // x: upper bound
            current_invariant,                               // y: actual invariant
        );

        // Call the gyro_e_math function
        let in_amount_big_int = gyro_e_math::calc_in_given_out(
            &balances,
            &amount_out_big_int,
            token_in_is_token0,
            &params,
            &derived,
            &invariant,
        )
        .ok()?;

        // Convert result back and apply fee
        let in_amount_sbfp = signed_fixed_point::SBfp::from_big_int(&in_amount_big_int).ok()?;
        // Convert I256 to U256 by extracting bytes (assuming positive result)
        if in_amount_sbfp.is_negative() {
            return None; // Cannot handle negative amounts in baseline solver
        }
        let mut bytes = [0u8; 32];
        in_amount_sbfp.as_i256().to_big_endian(&mut bytes);
        let in_amount_u256 = U256::from_big_endian(&bytes);
        let in_amount_bfp = Bfp::from_wei(in_amount_u256);
        let in_amount_downscaled = in_reserves.downscale_up(in_amount_bfp).ok()?;

        // Apply swap fee to get final amount
        add_swap_fee_amount(in_amount_downscaled, self.swap_fee).ok()
    }

    async fn gas_cost(&self) -> usize {
        GYRO_E_SWAP_GAS_COST
    }
}

impl GyroEPool {
    fn as_pool_ref(&self) -> GyroEPoolRef<'_> {
        GyroEPoolRef {
            reserves: &self.reserves,
            swap_fee: self.common.swap_fee,
            version: self.version,
            params_alpha: self.params_alpha,
            params_beta: self.params_beta,
            params_c: self.params_c,
            params_s: self.params_s,
            params_lambda: self.params_lambda,
            tau_alpha_x: self.tau_alpha_x,
            tau_alpha_y: self.tau_alpha_y,
            tau_beta_x: self.tau_beta_x,
            tau_beta_y: self.tau_beta_y,
            u: self.u,
            v: self.v,
            w: self.w,
            z: self.z,
            d_sq: self.d_sq,
        }
    }
}

impl BaselineSolvable for GyroEPool {
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

#[derive(Debug)]
pub struct ReClammPoolRef<'a> {
    pub reserves: &'a BTreeMap<H160, TokenState>,
    pub swap_fee: Bfp,
    pub last_virtual_balances: [Bfp; 2],
    pub daily_price_shift_base: Bfp,
    pub last_timestamp: u64,
    pub centeredness_margin: Bfp,
    pub start_fourth_root_price_ratio: Bfp,
    pub end_fourth_root_price_ratio: Bfp,
    pub price_ratio_update_start_time: u64,
    pub price_ratio_update_end_time: u64,
}

impl ReClammPoolRef<'_> {
    fn compute_virtuals_and_balances(
        &self,
        token0: H160,
        token1: H160,
        balances: &BTreeMap<H160, TokenState>,
    ) -> Option<([Bfp; 2], Bfp, Bfp, bool)> {
        let r0 = balances.get(&token0)?;
        let r1 = balances.get(&token1)?;
        let balances_scaled18 = [r0.upscaled_balance().ok()?, r1.upscaled_balance().ok()?];
        let prs = reclamm_math::PriceRatioState {
            price_ratio_update_start_time: self.price_ratio_update_start_time,
            price_ratio_update_end_time: self.price_ratio_update_end_time,
            start_fourth_root_price_ratio: self.start_fourth_root_price_ratio,
            end_fourth_root_price_ratio: self.end_fourth_root_price_ratio,
        };
        let (va, vb, changed) = reclamm_math::compute_current_virtual_balances(
            self.last_timestamp,
            &balances_scaled18,
            self.last_virtual_balances[0],
            self.last_virtual_balances[1],
            self.daily_price_shift_base,
            self.last_timestamp,
            self.centeredness_margin,
            prs,
        )
        .ok()?;
        Some((balances_scaled18, va, vb, changed))
    }

    fn get_amount_out_inner(
        &self,
        out_token: H160,
        in_amount: U256,
        in_token: H160,
    ) -> Option<U256> {
        let (token0, token1) = if in_token < out_token {
            (in_token, out_token)
        } else {
            (out_token, in_token)
        };
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        // Apply swap fee
        let in_amount_minus_fees = subtract_swap_fee_amount(in_amount, self.swap_fee).ok()?;

        let (balances_scaled18, va, vb, _changed) =
            self.compute_virtuals_and_balances(token0, token1, self.reserves)?;

        // Map token indices based on address ordering
        let (index_in, index_out) = if in_token == token0 {
            (0usize, 1usize)
        } else {
            (1usize, 0usize)
        };

        let amount_in_scaled18 = in_reserves.upscale(in_amount_minus_fees).ok()?;
        let out_scaled = reclamm_math::compute_out_given_in(
            &balances_scaled18,
            va,
            vb,
            index_in,
            index_out,
            amount_in_scaled18,
        )
        .ok()?;
        out_reserves.downscale_down(out_scaled).ok()
    }

    fn get_amount_in_inner(
        &self,
        in_token: H160,
        out_amount: U256,
        out_token: H160,
    ) -> Option<U256> {
        let (token0, token1) = if in_token < out_token {
            (in_token, out_token)
        } else {
            (out_token, in_token)
        };
        let in_reserves = self.reserves.get(&in_token)?;
        let out_reserves = self.reserves.get(&out_token)?;

        let (balances_scaled18, va, vb, _changed) =
            self.compute_virtuals_and_balances(token0, token1, self.reserves)?;

        let (index_in, index_out) = if in_token == token0 {
            (0usize, 1usize)
        } else {
            (1usize, 0usize)
        };

        let out_amount_scaled18 = out_reserves.upscale(out_amount).ok()?;
        let in_scaled = reclamm_math::compute_in_given_out(
            &balances_scaled18,
            va,
            vb,
            index_in,
            index_out,
            out_amount_scaled18,
        )
        .ok()?;
        let in_downscaled = in_reserves.downscale_up(in_scaled).ok()?;
        add_swap_fee_amount(in_downscaled, self.swap_fee).ok()
    }
}

impl ReClammPool {
    fn as_pool_ref(&self) -> ReClammPoolRef<'_> {
        ReClammPoolRef {
            reserves: &self.reserves,
            swap_fee: self.common.swap_fee,
            last_virtual_balances: [
                Bfp::from_wei(self.last_virtual_balances[0]),
                Bfp::from_wei(self.last_virtual_balances[1]),
            ],
            daily_price_shift_base: self.daily_price_shift_base,
            last_timestamp: self.last_timestamp,
            centeredness_margin: self.centeredness_margin,
            start_fourth_root_price_ratio: self.start_fourth_root_price_ratio,
            end_fourth_root_price_ratio: self.end_fourth_root_price_ratio,
            price_ratio_update_start_time: self.price_ratio_update_start_time,
            price_ratio_update_end_time: self.price_ratio_update_end_time,
        }
    }
}

impl BaselineSolvable for ReClammPool {
    async fn get_amount_out(&self, out_token: H160, input: (U256, H160)) -> Option<U256> {
        self.as_pool_ref()
            .get_amount_out_inner(out_token, input.0, input.1)
    }

    async fn get_amount_in(&self, in_token: H160, output: (U256, H160)) -> Option<U256> {
        self.as_pool_ref()
            .get_amount_in_inner(in_token, output.0, output.1)
    }

    async fn gas_cost(&self) -> usize {
        RECLAMM_SWAP_GAS_COST
    }
}

/// QuantAMM pool data as a reference used for computing input and output
/// amounts.
#[derive(Debug)]
pub struct QuantAmmPoolRef<'a> {
    pub reserves: &'a BTreeMap<H160, TokenState>,
    pub swap_fee: Bfp,
    pub max_trade_size_ratio: Bfp,
    pub first_four_weights_and_multipliers: &'a [I256],
    pub second_four_weights_and_multipliers: &'a [I256],
    pub last_update_time: u64,
    pub last_interop_time: u64,
    pub current_timestamp: u64,
}

impl QuantAmmPoolRef<'_> {
    fn get_amount_out_inner(
        &self,
        out_token: H160,
        amount_in: U256,
        in_token: H160,
    ) -> Option<U256> {
        // Get reserves
        let in_reserve = self.reserves.get(&in_token)?;
        let out_reserve = self.reserves.get(&out_token)?;

        // Get token indices
        let in_index = self.reserves.keys().position(|&token| token == in_token)?;
        let out_index = self.reserves.keys().position(|&token| token == out_token)?;

        // Apply swap fee first (subtract from input, like weighted pools)
        let amount_in_minus_fees = subtract_swap_fee_amount(amount_in, self.swap_fee).ok()?;

        // Extract weights and multipliers from packed arrays (matches balancer-maths
        // pattern)
        let (weights, multipliers) = extract_weights_and_multipliers(
            &self.first_four_weights_and_multipliers,
            &self.second_four_weights_and_multipliers,
            self.reserves.len(),
        )?;

        let upscaled_amount_in = in_reserve.upscale(amount_in_minus_fees).ok()?;

        // Check max trade size ratio for input (matches balancer-maths)
        let max_in_amount = in_reserve
            .upscaled_balance()
            .ok()?
            .mul_down(self.max_trade_size_ratio)
            .ok()?;
        if upscaled_amount_in > max_in_amount {
            return None; // MaxTradeSizeRatio exceeded
        }

        // Calculate interpolated weights for token pair (matches balancer-maths
        // _getNormalizedWeightPair)
        let (weight_in, weight_out) = quantamm_math::calculate_normalized_weight_pair(
            in_index,
            out_index,
            &weights,
            &multipliers,
            self.last_update_time,
            self.last_interop_time,
            self.current_timestamp,
        )
        .ok()?;

        // Use QuantAMM math functions (matches services pattern)
        let amount_out = quantamm_math::compute_out_given_in(
            in_reserve.upscaled_balance().ok()?,
            weight_in,
            out_reserve.upscaled_balance().ok()?,
            weight_out,
            upscaled_amount_in,
        )
        .ok()?;

        // Check max trade size ratio for output (matches balancer-maths)
        let max_out_amount = out_reserve
            .upscaled_balance()
            .ok()?
            .mul_down(self.max_trade_size_ratio)
            .ok()?;
        if amount_out > max_out_amount {
            return None; // MaxTradeSizeRatio exceeded
        }

        // Downscale result
        out_reserve.downscale_down(amount_out).ok()
    }

    fn get_amount_in_inner(
        &self,
        in_token: H160,
        amount_out: U256,
        out_token: H160,
    ) -> Option<U256> {
        // Get reserves
        let in_reserve = self.reserves.get(&in_token)?;
        let out_reserve = self.reserves.get(&out_token)?;

        // Get token indices
        let in_index = self.reserves.keys().position(|&token| token == in_token)?;
        let out_index = self.reserves.keys().position(|&token| token == out_token)?;

        // Extract weights and multipliers from packed arrays (matches balancer-maths
        // pattern)
        let (weights, multipliers) = extract_weights_and_multipliers(
            &self.first_four_weights_and_multipliers,
            &self.second_four_weights_and_multipliers,
            self.reserves.len(),
        )?;

        let upscaled_amount_out = out_reserve.upscale(amount_out).ok()?;

        // Check max trade size ratio for output (matches balancer-maths)
        let max_out_amount = out_reserve
            .upscaled_balance()
            .ok()?
            .mul_down(self.max_trade_size_ratio)
            .ok()?;
        if upscaled_amount_out > max_out_amount {
            return None; // MaxTradeSizeRatio exceeded
        }

        // Calculate interpolated weights for token pair (matches balancer-maths
        // _getNormalizedWeightPair)
        let (weight_in, weight_out) = quantamm_math::calculate_normalized_weight_pair(
            in_index,
            out_index,
            &weights,
            &multipliers,
            self.last_update_time,
            self.last_interop_time,
            self.current_timestamp,
        )
        .ok()?;

        // Use QuantAMM math functions (matches services pattern)
        let amount_in_before_fee = quantamm_math::compute_in_given_out(
            in_reserve.upscaled_balance().ok()?,
            weight_in,
            out_reserve.upscaled_balance().ok()?,
            weight_out,
            upscaled_amount_out,
        )
        .ok()?;

        // Check max trade size ratio for input (matches balancer-maths)
        let max_in_amount = in_reserve
            .upscaled_balance()
            .ok()?
            .mul_down(self.max_trade_size_ratio)
            .ok()?;
        if amount_in_before_fee > max_in_amount {
            return None; // MaxTradeSizeRatio exceeded
        }

        // Downscale and add swap fee (like weighted pools)
        let amount_in_raw = in_reserve.downscale_up(amount_in_before_fee).ok()?;
        add_swap_fee_amount(amount_in_raw, self.swap_fee).ok()
    }
}

impl BaselineSolvable for QuantAmmPoolRef<'_> {
    async fn get_amount_out(
        &self,
        out_token: H160,
        (amount_in, in_token): (U256, H160),
    ) -> Option<U256> {
        if amount_in.is_zero() {
            return Some(U256::zero());
        }
        self.get_amount_out_inner(out_token, amount_in, in_token)
    }

    async fn get_amount_in(
        &self,
        in_token: H160,
        (amount_out, out_token): (U256, H160),
    ) -> Option<U256> {
        if amount_out.is_zero() {
            return Some(U256::zero());
        }
        self.get_amount_in_inner(in_token, amount_out, out_token)
    }

    async fn gas_cost(&self) -> usize {
        // Approximate gas cost for QuantAMM swaps (higher than weighted due to weight
        // calculations)
        180_000
    }
}

impl QuantAmmPool {
    fn as_pool_ref(&self) -> QuantAmmPoolRef<'_> {
        QuantAmmPoolRef {
            reserves: &self.reserves,
            swap_fee: self.common.swap_fee,
            max_trade_size_ratio: self.max_trade_size_ratio,
            first_four_weights_and_multipliers: &self.first_four_weights_and_multipliers,
            second_four_weights_and_multipliers: &self.second_four_weights_and_multipliers,
            last_update_time: self.last_update_time,
            last_interop_time: self.last_interop_time,
            current_timestamp: self.current_timestamp,
        }
    }
}

impl BaselineSolvable for QuantAmmPool {
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
    use {super::*, crate::sources::balancer_v3::pool_fetching::CommonPoolState};

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
                        rate: U256::exp10(18),
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

    fn create_stable_pool_with(
        tokens: Vec<H160>,
        balances: Vec<U256>,
        amplification_parameter: AmplificationParameter,
        scaling_factors: Vec<Bfp>,
        swap_fee: U256,
    ) -> StablePool {
        let mut reserves = BTreeMap::new();
        for i in 0..tokens.len() {
            let (token, balance, scaling_factor) = (tokens[i], balances[i], scaling_factors[i]);
            reserves.insert(
                token,
                TokenState {
                    balance,
                    scaling_factor,
                    rate: U256::exp10(18),
                },
            );
        }
        StablePool {
            common: CommonPoolState {
                id: Default::default(),
                address: H160::zero(),
                swap_fee: Bfp::from_wei(swap_fee),
                paused: true,
            },
            reserves,
            amplification_parameter,
            version: Default::default(),
        }
    }

    #[test]
    fn downscale() {
        let token_state = TokenState {
            balance: Default::default(),
            scaling_factor: Bfp::exp10(12),
            rate: U256::exp10(18),
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

    #[test]
    fn construct_balances_and_token_indices() {
        let tokens: Vec<_> = (1..=3).map(H160::from_low_u64_be).collect();
        let balances = (1..=3).map(|n| n.into()).collect();
        let pool = create_stable_pool_with(
            tokens.clone(),
            balances,
            AmplificationParameter::try_new(1.into(), 1.into()).unwrap(),
            vec![Bfp::exp10(18), Bfp::exp10(18), Bfp::exp10(18)],
            1.into(),
        );

        for token_i in tokens.iter() {
            for token_j in tokens.iter() {
                let res_ij = pool
                    .as_pool_ref()
                    .upscale_balances_with_token_indices(token_i, token_j)
                    .unwrap();
                assert_eq!(
                    res_ij.balances[res_ij.token_index_in],
                    pool.reserves
                        .get(token_i)
                        .unwrap()
                        .upscaled_balance()
                        .unwrap()
                );
                assert_eq!(
                    res_ij.balances[res_ij.token_index_out],
                    pool.reserves
                        .get(token_j)
                        .unwrap()
                        .upscaled_balance()
                        .unwrap()
                );
            }
        }
    }

    #[tokio::test]
    async fn stable_get_amount_out() {
        // Test based on actual swap.
        // https://dashboard.tenderly.co/tx/main/0x75be93fff064ad46b423b9e20cee09b0ae7f741087f43e4187d4f4cf59f54229/debugger
        // Token addresses are irrelevant for computation.
        let dai = H160::from_low_u64_be(1);
        let usdc = H160::from_low_u64_be(2);
        let tusd = H160::from_low_u64_be(3);
        let tokens = vec![dai, usdc, tusd];
        let scaling_exps = vec![Bfp::exp10(0), Bfp::exp10(12), Bfp::exp10(12)];
        let amplification_parameter =
            AmplificationParameter::try_new(570000.into(), 1000.into()).unwrap();
        let balances = vec![
            40_927_687_702_846_622_465_144_342_i128.into(),
            59_448_574_675_062_i128.into(),
            55_199_308_926_456_i128.into(),
        ];
        let swap_fee_percentage = 300_000_000_000_000u128.into();
        let pool = create_stable_pool_with(
            tokens,
            balances,
            amplification_parameter,
            scaling_exps,
            swap_fee_percentage,
        );
        // Etherscan for amount verification:
        // https://etherscan.io/tx/0x75be93fff064ad46b423b9e20cee09b0ae7f741087f43e4187d4f4cf59f54229
        let amount_in = 1_886_982_823_746_269_817_650_i128.into();
        let amount_out = 1_887_770_905_i128;
        let res_out = pool.get_amount_out(usdc, (amount_in, dai)).await;
        assert_eq!(res_out.unwrap(), amount_out.into());
    }

    #[tokio::test]
    async fn stable_get_amount_in() {
        // Test based on actual swap.
        // https://dashboard.tenderly.co/tx/main/0x38487122158eef6b63570b5d3754ddc223c63af5c049d7b80acacb9e8ca89a63/debugger
        // Token addresses are irrelevant for computation.
        let dai = H160::from_low_u64_be(1);
        let usdc = H160::from_low_u64_be(2);
        let tusd = H160::from_low_u64_be(3);
        let tokens = vec![dai, usdc, tusd];
        let scaling_exps = vec![Bfp::exp10(0), Bfp::exp10(12), Bfp::exp10(12)];
        let amplification_parameter =
            AmplificationParameter::try_new(570000.into(), 1000.into()).unwrap();
        let balances = vec![
            34_869_494_603_218_073_631_628_580_i128.into(),
            48_176_005_970_419_i128.into(),
            44_564_350_355_030_i128.into(),
        ];
        let swap_fee_percentage = 300_000_000_000_000u128.into();
        let pool = create_stable_pool_with(
            tokens,
            balances,
            amplification_parameter,
            scaling_exps,
            swap_fee_percentage,
        );
        // Etherscan for amount verification:
        // https://etherscan.io/tx/0x38487122158eef6b63570b5d3754ddc223c63af5c049d7b80acacb9e8ca89a63
        let amount_in = 900_816_325_i128;
        let amount_out = 900_000_000_000_000_000_000_u128.into();
        let res_out = pool.get_amount_in(usdc, (amount_out, dai)).await;
        assert_eq!(res_out.unwrap(), amount_in.into());
    }
}

/// Extract weights and multipliers from packed arrays.
/// This matches the getFirstFourWeightsAndMultipliers and
/// getSecondFourWeightsAndMultipliers pattern from balancer-maths.
fn extract_weights_and_multipliers(
    first_four: &[I256],
    second_four: &[I256],
    num_tokens: usize,
) -> Option<(Vec<I256>, Vec<I256>)> {
    let mut weights = Vec::new();
    let mut multipliers = Vec::new();

    // Process first four tokens (matches balancer-maths
    // getFirstFourWeightsAndMultipliers)
    let first_token_count = std::cmp::min(4, num_tokens);
    for i in 0..first_token_count {
        if i < first_four.len() / 2 {
            weights.push(first_four[i]);
            if i + first_token_count < first_four.len() {
                multipliers.push(first_four[i + first_token_count]);
            } else {
                multipliers.push(I256::zero());
            }
        }
    }

    // Process remaining tokens if any (matches balancer-maths
    // getSecondFourWeightsAndMultipliers)
    if num_tokens > 4 {
        let remaining_count = num_tokens - 4;
        for i in 0..remaining_count {
            if i < second_four.len() / 2 {
                weights.push(second_four[i]);
                if i + remaining_count < second_four.len() {
                    multipliers.push(second_four[i + remaining_count]);
                } else {
                    multipliers.push(I256::zero());
                }
            }
        }
    }

    Some((weights, multipliers))
}
