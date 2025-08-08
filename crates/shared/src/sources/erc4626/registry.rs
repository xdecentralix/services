use ethcontract::H160;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, Deserialize)]
pub struct Erc4626Config {
    pub enabled: bool,
    pub vaults: Vec<H160>,
}

#[derive(Debug, Clone)]
pub struct VaultMeta {
    pub vault: H160,
    pub asset: H160, // resolved via IERC4626.asset() on first use
    pub epsilon_bps: u16, // small internal default (e.g., 3–5 bps)
}

#[derive(Debug, Clone)]
pub struct Erc4626Registry {
    enabled: bool,
    vaults: Vec<H160>,
    cache: parking_lot::RwLock<HashMap<H160, VaultMeta>>,
    web3: ethcontract::Web3,
}

impl Erc4626Registry {
    pub fn new(cfg: Erc4626Config, web3: ethcontract::Web3) -> Self {
        Self { enabled: cfg.enabled, vaults: cfg.vaults, cache: Default::default(), web3 }
    }

    pub fn enabled(&self) -> bool { self.enabled }

    // Resolve on first use; cache result.
    pub async fn get(&self, vault: H160) -> Option<VaultMeta> {
        if !self.enabled || !self.vaults.contains(&vault) { return None; }
        if let Some(m) = self.cache.read().get(&vault).cloned() { return Some(m); }

        let ierc4626 = contracts::IERC4626::at(&self.web3, vault);
        let asset = ierc4626.asset().call().await.ok()?;
        let meta = VaultMeta { vault, asset, epsilon_bps: 5 };
        self.cache.write().insert(vault, meta.clone());
        Some(meta)
    }

    pub async fn all(&self) -> Vec<VaultMeta> {
        let mut out = Vec::new();
        for &v in &self.vaults {
            if let Some(m) = self.get(v).await { out.push(m); }
        }
        out
    }
}