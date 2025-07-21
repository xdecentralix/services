//! Balancer pool registry initialization.
//!
//! This module contains a component used to initialize Balancer pool registries
//! with existing data in order to reduce the "cold start" time of the service.

use {
    super::graph_api::{BalancerApiClient, RegisteredPools},
    anyhow::Result,
};

#[async_trait::async_trait]
pub trait PoolInitializing: Send + Sync {
    async fn initialize_pools(&self) -> Result<RegisteredPools>;
}

#[async_trait::async_trait]
impl PoolInitializing for BalancerApiClient {
    async fn initialize_pools(&self) -> Result<RegisteredPools> {
        let registered_pools = self.get_registered_pools().await?;
        tracing::debug!(
            block = %registered_pools.fetched_block_number, pools = %registered_pools.pools.len(),
            "initialized {} V2 pools from Balancer API v3",
            registered_pools.pools.len()
        );
        Ok(registered_pools)
    }
}
