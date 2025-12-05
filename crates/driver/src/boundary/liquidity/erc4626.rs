use {
    crate::{
        boundary::Result,
        domain::{eth, liquidity},
        infra::blockchain::Ethereum,
    },
    anyhow::Result as AnyResult,
    chain::Chain,
    ethrpc::alloy::conversions::IntoLegacy,
    shared::sources::erc4626::registry::Erc4626Registry,
    solver::{
        liquidity::erc4626::{Erc4626LiquiditySource, Erc4626Order},
        liquidity_collector::{BackgroundInitLiquiditySource, LiquidityCollecting},
    },
    std::time::Duration,
};

/// Maps chain names to their directory names in the configs folder
fn chain_to_config_dir(chain: &Chain) -> &'static str {
    match chain {
        Chain::ArbitrumOne => "arbitrum",
        Chain::Mainnet => "mainnet",
        Chain::Goerli => "goerli",
        Chain::Sepolia => "sepolia",
        Chain::Gnosis => "gnosis",
        Chain::Base => "base",
        Chain::Bnb => "bnb",
        Chain::Avalanche => "avalanche",
        Chain::Optimism => "optimism",
        Chain::Polygon => "polygon",
        Chain::Hardhat => "hardhat",
        Chain::Lens => "lens",
        Chain::Linea => "linea",
        Chain::Plasma => "plasma",
    }
}

/// Returns the ERC4626 vault addresses from the config file if enabled.
/// These should be added to base tokens to enable routing through vaults.
pub fn get_vault_addresses(chain: &Chain) -> Vec<eth::H160> {
    let config_dir = chain_to_config_dir(chain);
    let primary_path = format!("configs/{}/erc4626.toml", config_dir);
    let fallback_path = format!("../{}", primary_path);

    let config = shared::sources::erc4626::registry::load_config_from_file(std::path::Path::new(
        &primary_path,
    ))
    .or_else(|_| {
        shared::sources::erc4626::registry::load_config_from_file(std::path::Path::new(
            &fallback_path,
        ))
    });

    match config {
        Ok(cfg) if cfg.enabled => {
            tracing::debug!(
                vault_count = cfg.vaults.len(),
                "Adding ERC4626 vault tokens to base tokens"
            );
            cfg.vaults.into_iter().map(eth::H160::from).collect()
        }
        _ => {
            tracing::debug!("ERC4626 not enabled; no vault tokens added to base tokens");
            vec![]
        }
    }
}

/// Builds the ERC4626 liquidity collector if enabled via
/// configs/<chain>/erc4626.toml.
pub async fn maybe_collector(eth: &Ethereum) -> AnyResult<Vec<Box<dyn LiquidityCollecting>>> {
    // Try to load per-chain config file; if missing or disabled, return empty.
    let chain = eth.chain();
    let config_dir = chain_to_config_dir(&chain);
    let primary_path = format!("configs/{}/erc4626.toml", config_dir);
    let fallback_path = format!("../{}", primary_path);
    let web3 = eth.web3().clone();
    let settlement = eth.contracts().settlement().clone();
    let registry: Erc4626Registry = match shared::sources::erc4626::registry::registry_from_file(
        std::path::Path::new(&primary_path),
        web3.clone(),
    ) {
        Ok(reg) if reg.enabled() => {
            tracing::debug!(path = %primary_path, "Loaded ERC4626 registry from config file");
            reg
        }
        _ => {
            // Try fallback path when running from `services/` as CWD
            match shared::sources::erc4626::registry::registry_from_file(
                std::path::Path::new(&fallback_path),
                web3.clone(),
            ) {
                Ok(reg) if reg.enabled() => {
                    tracing::debug!(path = %fallback_path, "Loaded ERC4626 registry from fallback config file");
                    reg
                }
                _ => {
                    tracing::debug!(
                        primary = %primary_path,
                        fallback = %fallback_path,
                        "ERC4626 registry disabled or config file not found; skipping source"
                    );
                    return Ok(vec![]);
                }
            }
        }
    };

    let source = Erc4626LiquiditySource {
        web3,
        settlement,
        registry,
    };
    let init = move || {
        let source = source.clone();
        async move {
            tracing::debug!("initializing ERC4626 liquidity source");
            Ok(source)
        }
    };
    let collector =
        BackgroundInitLiquiditySource::new("erc4626", init, Duration::from_secs(5), None);
    Ok(vec![Box::new(collector)])
}

pub fn to_domain(id: liquidity::Id, order: Erc4626Order) -> Result<liquidity::Liquidity> {
    // Extract vault and asset addresses explicitly from the order.
    // The wrap/unwrap variants contain the actual contract references with correct
    // semantics.
    let (vault, asset) = if let Some(ref wrap) = order.wrap {
        // Wrap order: asset -> vault
        let vault_addr: eth::H160 = wrap.vault.address().into();
        let asset_addr: eth::H160 = wrap.underlying.address().into();
        (vault_addr, asset_addr)
    } else if let Some(ref unwrap) = order.unwrap {
        // Unwrap order: vault -> asset
        // The vault address is known, derive asset from TokenPair
        let vault_addr: eth::H160 = unwrap.vault.address().into();
        let (a, b) = order.tokens.get();
        let a_h160: eth::H160 = a.into_legacy();
        let b_h160: eth::H160 = b.into_legacy();
        // The asset is whichever token in the pair is NOT the vault
        let asset_addr = if a_h160 == vault_addr { b_h160 } else { a_h160 };
        (vault_addr, asset_addr)
    } else {
        // Fallback: shouldn't happen, but handle gracefully
        let (a, b) = order.tokens.get();
        (a.into_legacy(), b.into_legacy())
    };

    Ok(liquidity::Liquidity {
        id,
        gas: 90_000u64.into(),
        kind: liquidity::Kind::Erc4626(liquidity::erc4626::Edge {
            vault: eth::TokenAddress(vault.into()),
            asset: eth::TokenAddress(asset.into()),
        }),
    })
}

pub fn to_wrap_interaction(
    _input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> Result<eth::Interaction> {
    // encode IERC4626.mint(shares_out, receiver)
    let selector = hex_literal::hex!("94bf804d"); // mint(uint256,address)
    let mut shares = [0u8; 32];
    output.0.amount.0.to_big_endian(&mut shares);
    // Note: _input is intentionally not used here; it's used for bounded approval
    // generation elsewhere.
    tracing::debug!(
        shares_out = ?output.0.amount.0,
        receiver = ?receiver.0,
        target = ?output.0.token.0,
        "Encoding ERC4626 wrap interaction (mint)"
    );
    Ok(eth::Interaction {
        target: output.0.token.0.into(), // vault address as target
        value: eth::U256::zero().into(),
        call_data: [
            selector.as_slice(),
            &shares,
            [0; 12].as_slice(),
            receiver.0.as_bytes(),
        ]
        .concat()
        .into(),
    })
}

pub fn to_unwrap_interaction(
    _input: &liquidity::MaxInput,
    output: &liquidity::ExactOutput,
    receiver: &eth::Address,
) -> Result<eth::Interaction> {
    // encode IERC4626.withdraw(assets_out, receiver, owner)
    let selector = hex_literal::hex!("b460af94"); // withdraw(uint256,address,address)
    let mut assets = [0u8; 32];
    output.0.amount.0.to_big_endian(&mut assets);
    tracing::debug!(
        assets_out = ?output.0.amount.0,
        receiver = ?receiver.0,
        target = ?output.0.token.0,
        "Encoding ERC4626 unwrap interaction (withdraw)"
    );
    Ok(eth::Interaction {
        target: output.0.token.0.into(), // vault or asset? For withdraw target is vault
        value: eth::U256::zero().into(),
        call_data: [
            selector.as_slice(),
            &assets,
            [0; 12].as_slice(),
            receiver.0.as_bytes(),
            [0; 12].as_slice(),
            receiver.0.as_bytes(),
        ]
        .concat()
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use {super::*, crate::domain::eth};

    #[test]
    fn encode_wrap_and_unwrap() {
        let input = liquidity::MaxInput(eth::Asset {
            token: eth::H160::zero().into(),
            amount: 123.into(),
        });
        let output = liquidity::ExactOutput(eth::Asset {
            token: eth::H160::repeat_byte(0x11).into(),
            amount: 456.into(),
        });
        let receiver = &eth::Address(eth::H160::repeat_byte(0x22));

        let wrap = to_wrap_interaction(&input, &output, receiver).unwrap();
        assert_eq!(&wrap.call_data.0[0..4], &hex_literal::hex!("94bf804d"));

        let unwrap = to_unwrap_interaction(&input, &output, receiver).unwrap();
        assert_eq!(&unwrap.call_data.0[0..4], &hex_literal::hex!("b460af94"));
    }
}
