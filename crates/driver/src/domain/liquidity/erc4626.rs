use crate::domain::eth;

#[derive(Debug, Clone, Copy)]
pub struct Edge {
    /// The ERC4626 vault contract address
    pub vault: eth::TokenAddress,
    /// The underlying asset token address
    pub asset: eth::TokenAddress,
}
