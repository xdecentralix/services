//! Balancer V3 pool registry initialization.
//!
//! This module contains a component used to initialize Balancer V3 pool
//! registries with existing data in order to reduce the "cold start" time of
//! the service.

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

        // Log the first 10 pool IDs with full details
        let pool_count = registered_pools.pools.len();
        tracing::info!("initialized {} V3 pools from Balancer API v3", pool_count);

        // Log first 10 pools with full addresses
        if pool_count > 0 {
            let first_10_pools = registered_pools.pools.iter().take(10);
            for (i, pool) in first_10_pools.enumerate() {
                tracing::info!(
                    "V3 Pool {}: address={:?}, type={}, tokens={}",
                    i + 1,
                    pool.address,
                    pool.pool_type,
                    pool.pool_tokens.len()
                );
            }

            if pool_count > 10 {
                tracing::info!("... and {} more V3 pools", pool_count - 10);
            }
        }

        tracing::debug!(
            block = %registered_pools.fetched_block_number,
            pools = %pool_count,
            "V3 pool initialization complete"
        );

        Ok(registered_pools)
    }
}
