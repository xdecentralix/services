use {
    crate::{
        boundary::{self, Result},
        domain::{eth, liquidity},
        infra::blockchain::Ethereum,
    },
    anyhow::Result as AnyResult,
    shared::sources::erc4626::registry::Erc4626Registry,
    solver::{
        liquidity::erc4626::{Erc4626LiquiditySource, Erc4626Order},
        liquidity_collector::{BackgroundInitLiquiditySource, LiquidityCollecting},
    },
    std::time::Duration,
};

/// Builds the ERC4626 liquidity collector if enabled via
/// configs/<chain>/erc4626.toml.
pub async fn maybe_collector(eth: &Ethereum) -> AnyResult<Vec<Box<dyn LiquidityCollecting>>> {
    // Try to load per-chain config file; if missing or disabled, return empty.
    let chain = eth.chain();
    let chain_str = format!("{:?}", chain);
    let chain_lc = chain_str.to_lowercase();
    let primary_path = format!("configs/{}/erc4626.toml", chain_lc);
    let fallback_path = format!("../{}", primary_path);
    let web3 = boundary::web3(eth);
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
    // At this stage, amounts are populated during route realization; here we only
    // carry tokens and handler wiring
    let (a, b) = order.tokens.get();
    Ok(liquidity::Liquidity {
        id,
        gas: 90_000u64.into(),
        kind: liquidity::Kind::Erc4626(liquidity::erc4626::Edge {
            tokens: (eth::TokenAddress(a.into()), eth::TokenAddress(b.into())),
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
