use crate::domain::eth;

#[derive(Clone, Debug)]
pub struct Edge {
    pub asset: eth::TokenAddress,
    pub vault: eth::TokenAddress,
}
