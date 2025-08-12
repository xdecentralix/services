use {
    ethereum_types::{H160, U256},
    shared::{baseline_solver::BaselineSolvable, ethrpc::Web3},
};

/// Boundary ERC4626 edge that quotes via IERC4626 preview functions.
#[derive(Clone, Debug)]
pub struct Edge {
    pub vault: H160,
    pub asset: H160,
    contract: contracts::IERC4626,
}

impl Edge {
    pub fn new(web3: &Web3, vault: H160, asset: H160) -> Self {
        let contract = contracts::IERC4626::at(web3, vault);
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
                this.contract.preview_deposit(in_amount).call().await.ok()
            } else if in_token == this.vault && out_token == this.asset {
                // vault -> asset
                this.contract.preview_redeem(in_amount).call().await.ok()
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
                this.contract.preview_mint(out_amount).call().await.ok()
            } else if in_token == this.vault && out_token == this.asset {
                // vault -> asset (exact assets out)
                this.contract.preview_withdraw(out_amount).call().await.ok()
            } else {
                None
            }
        }
    }

    fn gas_cost(&self) -> impl std::future::Future<Output = usize> + Send {
        async { 90_000usize }
    }
}
