use {
    contracts::alloy::IERC4626,
    ethereum_types::{H160, U256},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    shared::{baseline_solver::BaselineSolvable, ethrpc::Web3},
};

/// Boundary ERC4626 edge that quotes via IERC4626 preview functions.
#[derive(Clone, Debug)]
pub struct Edge {
    pub vault: H160,
    pub asset: H160,
    contract: IERC4626::Instance,
}

impl Edge {
    pub fn new(web3: &Web3, vault: H160, asset: H160) -> Self {
        let contract = IERC4626::Instance::new(vault.into_alloy(), web3.alloy.clone());
        Self {
            vault,
            asset,
            contract,
        }
    }
}

impl BaselineSolvable for Edge {
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
            if in_token == this.asset && out_token == this.vault {
                // asset -> vault
                this.contract
                    .previewDeposit(in_amount.into_alloy())
                    .call()
                    .await
                    .ok()
                    .map(|r| r.into_legacy())
            } else if in_token == this.vault && out_token == this.asset {
                // vault -> asset
                this.contract
                    .previewRedeem(in_amount.into_alloy())
                    .call()
                    .await
                    .ok()
                    .map(|r| r.into_legacy())
            } else {
                None
            }
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
            if in_token == this.asset && out_token == this.vault {
                // asset -> vault (exact shares out)
                this.contract
                    .previewMint(out_amount.into_alloy())
                    .call()
                    .await
                    .ok()
                    .map(|r| r.into_legacy())
            } else if in_token == this.vault && out_token == this.asset {
                // vault -> asset (exact assets out)
                this.contract
                    .previewWithdraw(out_amount.into_alloy())
                    .call()
                    .await
                    .ok()
                    .map(|r| r.into_legacy())
            } else {
                None
            }
        }
    }

    async fn gas_cost(&self) -> usize {
        90_000usize
    }
}
