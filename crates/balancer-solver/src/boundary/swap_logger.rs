//! Swap logging module for debugging and verification purposes.
//!
//! This module provides non-intrusive logging of all swap calculations
//! performed during solving. The logs can be used to verify outputs against
//! on-chain contracts.

use {
    ethereum_types::{H160, U256},
    serde::{Deserialize, Serialize},
    std::sync::{Arc, Mutex},
};

/// A single swap calculation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapRecord {
    /// Pool/liquidity ID
    pub liquidity_id: String,

    /// Pool kind (e.g., "weightedProduct", "stable", "gyroE")
    pub kind: String,

    /// Pool address
    pub address: String,

    /// Input token address
    pub input_token: String,

    /// Input amount (in wei)
    pub input_amount: String,

    /// Output token address
    pub output_token: String,

    /// Calculated output amount (in wei), None if calculation failed
    pub output_amount: Option<String>,

    /// Pool-specific parameters
    pub pool_params: serde_json::Value,
}

/// Thread-safe swap logger that collects swap records during solving
#[derive(Clone)]
pub struct SwapLogger {
    records: Arc<Mutex<Vec<SwapRecord>>>,
}

impl SwapLogger {
    /// Create a new swap logger
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Log a swap calculation
    pub fn log_swap(&self, record: SwapRecord) {
        if let Ok(mut records) = self.records.lock() {
            records.push(record);
        }
    }

    /// Get all logged swap records
    pub fn get_records(&self) -> Vec<SwapRecord> {
        self.records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default()
    }

    /// Get the number of logged swaps
    pub fn count(&self) -> usize {
        self.records
            .lock()
            .map(|records| records.len())
            .unwrap_or(0)
    }
}

impl Default for SwapLogger {
    fn default() -> Self {
        Self::new()
    }
}
