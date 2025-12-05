use crate::domain::eth;

#[derive(Debug, Clone, Copy)]
pub struct Edge {
    /// Underlying ERC20 token of the vault
    pub asset: eth::TokenAddress,
    /// ERC4626 vault token
    pub vault: eth::TokenAddress,
}
