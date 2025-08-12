use {
    ethcontract::H160,
    serde::Deserialize,
    std::{
        collections::HashMap,
        path::Path,
        sync::{Arc, RwLock},
    },
};

#[derive(Debug, Clone, Deserialize)]
pub struct Erc4626Config {
    pub enabled: bool,
    pub vaults: Vec<H160>,
}

#[derive(Debug, Clone)]
pub struct VaultMeta {
    pub vault: H160,
    pub asset: H160,      // resolved via IERC4626.asset() on first use
    pub epsilon_bps: u16, // small internal default (e.g., 3–5 bps)
}

#[derive(Debug, Clone)]
pub struct Erc4626Registry {
    enabled: bool,
    vaults: Vec<H160>,
    cache: Arc<RwLock<HashMap<H160, VaultMeta>>>,
    web3: crate::ethrpc::Web3,
}

impl Erc4626Registry {
    pub fn new(cfg: Erc4626Config, web3: crate::ethrpc::Web3) -> Self {
        Self {
            enabled: cfg.enabled,
            vaults: cfg.vaults,
            cache: Arc::new(Default::default()),
            web3,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    // Resolve on first use; cache result.
    pub async fn get(&self, vault: H160) -> Option<VaultMeta> {
        if !self.enabled || !self.vaults.contains(&vault) {
            return None;
        }
        if let Some(m) = self.cache.read().unwrap().get(&vault).cloned() {
            return Some(m);
        }

        let ierc4626 = contracts::IERC4626::at(&self.web3, vault);
        let asset = ierc4626.asset().call().await.ok()?;
        let meta = VaultMeta {
            vault,
            asset,
            epsilon_bps: 5,
        };
        self.cache.write().unwrap().insert(vault, meta.clone());
        Some(meta)
    }

    pub async fn all(&self) -> Vec<VaultMeta> {
        let mut out = Vec::new();
        for &v in &self.vaults {
            if let Some(m) = self.get(v).await {
                out.push(m);
            }
        }
        out
    }
}

/// Load an `Erc4626Config` from a TOML file located at `path`.
pub fn load_config_from_file(path: &Path) -> anyhow::Result<Erc4626Config> {
    let text = std::fs::read_to_string(path)?;
    let cfg: Erc4626Config = toml::from_str(&text)?;
    Ok(cfg)
}

/// Construct a registry from a TOML config file path and a `web3` instance.
pub fn registry_from_file(
    path: &Path,
    web3: crate::ethrpc::Web3,
) -> anyhow::Result<Erc4626Registry> {
    let cfg = load_config_from_file(path)?;
    Ok(Erc4626Registry::new(cfg, web3))
}
