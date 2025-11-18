//! ERC4626 wrap/unwrap edges integrated into the baseline solver.

pub mod registry;

use {
    crate::{
        baseline_solver::BaselineSolvable,
        ethrpc::Web3,
        sources::erc4626::registry::{Erc4626Registry, VaultMeta},
    },
    ethcontract::{H160, U256},
    ethrpc::alloy::conversions::IntoAlloy,
    model::TokenPair,
    std::collections::HashMap,
};

/// Default epsilon (in basis points) applied pessimistically to exact-out
/// previews.
const DEFAULT_EPSILON_BPS: u16 = 5; // 0.05%

/// A directed ERC4626 edge between an underlying asset and its vault token.
#[derive(Clone)]
pub struct Erc4626Edge {
    pub vault: H160,
    pub asset: H160,
    pub epsilon_bps: u16,
    contract: contracts::IERC4626,
}

impl Erc4626Edge {
    pub fn new(web3: &Web3, meta: &VaultMeta) -> Self {
        let contract = contracts::IERC4626::at(web3, meta.vault);
        Self {
            vault: meta.vault,
            asset: meta.asset,
            epsilon_bps: meta.epsilon_bps,
            contract,
        }
    }
}

fn apply_epsilon_ceiled(amount: U256, epsilon_bps: u16) -> U256 {
    // ceil(amount * (10_000 + eps) / 10_000)
    let numerator = amount.saturating_mul(U256::from(10_000u64 + epsilon_bps as u64));
    numerator
        .saturating_add(U256::from(10_000u64 - 1))
        .checked_div(U256::from(10_000u64))
        .unwrap_or(numerator) // fallback, though division by 10_000 won't fail
}

impl BaselineSolvable for Erc4626Edge {
    fn get_amount_out(
        &self,
        out_token: H160,
        (in_amount, in_token): (U256, H160),
    ) -> impl std::future::Future<Output = Option<U256>> + Send {
        let this = self.clone();
        async move {
            if in_amount.is_zero() {
                return Some(U256::zero());
            }

            // Wrap (asset -> vault): use previewDeposit
            if in_token == this.asset && out_token == this.vault {
                let res = this.contract.preview_deposit(in_amount).call().await.ok();
                if let Some(ref shares_out) = res {
                    tracing::debug!(
                        asset = ?this.asset,
                        vault = ?this.vault,
                        assets_in = ?in_amount,
                        shares_out = ?shares_out,
                        "ERC4626 get_amount_out wrap: preview_deposit"
                    );
                }
                return res;
            }

            // Unwrap (vault -> asset): use previewRedeem
            if in_token == this.vault && out_token == this.asset {
                let res = this.contract.preview_redeem(in_amount).call().await.ok();
                if let Some(ref assets_out) = res {
                    tracing::debug!(
                        vault = ?this.vault,
                        asset = ?this.asset,
                        shares_in = ?in_amount,
                        assets_out = ?assets_out,
                        "ERC4626 get_amount_out unwrap: preview_redeem"
                    );
                }
                return res;
            }

            None
        }
    }

    fn get_amount_in(
        &self,
        in_token: H160,
        (out_amount, out_token): (U256, H160),
    ) -> impl std::future::Future<Output = Option<U256>> + Send {
        let this = self.clone();
        async move {
            if out_amount.is_zero() {
                return Some(U256::zero());
            }

            // Wrap exact-out (asset -> vault): assets_in_max = ceil(previewMint(shares_out)
            // * (1+ε))
            if in_token == this.asset && out_token == this.vault {
                let preview = this.contract.preview_mint(out_amount).call().await.ok()?;
                let needed = apply_epsilon_ceiled(preview, this.epsilon_bps);
                tracing::debug!(
                    asset = ?this.asset,
                    vault = ?this.vault,
                    shares_out = ?out_amount,
                    assets_preview = ?preview,
                    epsilon_bps = this.epsilon_bps,
                    assets_in_max = ?needed,
                    "ERC4626 get_amount_in wrap exact-out: preview_mint with epsilon"
                );
                return Some(needed);
            }

            // Unwrap exact-out (vault -> asset): shares_in_max =
            // ceil(previewWithdraw(assets_out) * (1+ε))
            if in_token == this.vault && out_token == this.asset {
                let preview = this
                    .contract
                    .preview_withdraw(out_amount)
                    .call()
                    .await
                    .ok()?;
                let needed = apply_epsilon_ceiled(preview, this.epsilon_bps);
                tracing::debug!(
                    vault = ?this.vault,
                    asset = ?this.asset,
                    assets_out = ?out_amount,
                    shares_preview = ?preview,
                    epsilon_bps = this.epsilon_bps,
                    shares_in_max = ?needed,
                    "ERC4626 get_amount_in unwrap exact-out: preview_withdraw with epsilon"
                );
                return Some(needed);
            }

            None
        }
    }

    async fn gas_cost(&self) -> usize {
        90_000usize
    }
}

/// Build ERC4626 edges from the allowlisted registry.
pub async fn build_edges(
    web3: &Web3,
    registry: &Erc4626Registry,
) -> HashMap<TokenPair, Vec<Erc4626Edge>> {
    let mut map: HashMap<TokenPair, Vec<Erc4626Edge>> = HashMap::new();
    if !registry.enabled() {
        tracing::debug!("ERC4626 registry disabled; no edges built");
        return map;
    }

    // Gather all allowlisted vaults and create both directed edges per vault.
    let metas: Vec<VaultMeta> = registry.all().await;
    tracing::debug!(vault_count = metas.len(), "ERC4626 registry loaded vaults");
    for meta in metas {
        let meta = VaultMeta {
            epsilon_bps: if meta.epsilon_bps == 0 {
                DEFAULT_EPSILON_BPS
            } else {
                meta.epsilon_bps
            },
            ..meta
        };
        let edge = Erc4626Edge::new(web3, &meta);

        if let Some(pair) = TokenPair::new(meta.asset.into_alloy(), meta.vault.into_alloy()) {
            map.entry(pair).or_default().push(edge.clone());
        }
        if let Some(pair) = TokenPair::new(meta.vault.into_alloy(), meta.asset.into_alloy()) {
            map.entry(pair).or_default().push(edge);
        }

        tracing::debug!(
            vault = %meta.vault,
            asset = %meta.asset,
            epsilon_bps = meta.epsilon_bps,
            "Built ERC4626 edges for vault"
        );
    }

    let edge_count: usize = map.values().map(|v| v.len()).sum();
    tracing::debug!(edge_count, "ERC4626 edges built");
    map
}

#[cfg(test)]
mod tests {
    use primitive_types::U256;

    #[tokio::test]
    async fn epsilon_applied_via_get_amount_in() {
        let amount = U256::from(1000u64);
        let res = super::apply_epsilon_ceiled(amount, 5);
        assert_eq!(res, U256::from(1001u64));
    }
}
