//! This script is used to vendor Truffle JSON artifacts to be used for code
//! generation with `ethcontract`. This is done instead of fetching contracts
//! at build time to reduce the risk of failure.

use {
    anyhow::Result,
    contracts::paths,
    ethcontract_generate::Source,
    serde_json::{Map, Value},
    std::{
        fs,
        path::{Path, PathBuf},
    },
    tracing_subscriber::EnvFilter,
};

fn main() {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("LOG_FILTER").unwrap_or_else(|_| "warn,vendor=info".into()),
        )
        .init();

    if let Err(err) = run() {
        tracing::error!("Error vendoring contracts: {:?}", err);
        std::process::exit(-1);
    }
}

#[rustfmt::skip]
fn run() -> Result<()> {
    let vendor = Vendor::try_new()?;

    const ETHFLOW_VERSION: &str = "0.0.0-rc.3";
    // Balancer V2 contracts - Full
    vendor
        .full()
        .github(
            "BalancerV2Authorizer",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/tasks/20210418-authorizer/artifact/Authorizer.json",
        )?
        .github(
            "BalancerV2Vault",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/tasks/20210418-vault/artifact/Vault.json",
        )?
        .github(
            "BalancerV2WeightedPoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20210418-weighted-pool/artifact/WeightedPoolFactory.json",
        )?
        .github(
            "BalancerV2WeightedPool2TokensFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20210418-weighted-pool/artifact/WeightedPool2TokensFactory.json",
        )?
        .github(
            "BalancerV2WeightedPoolFactoryV3",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20230206-weighted-pool-v3/artifact/WeightedPoolFactory.json",
        )?
        .github(
            "BalancerV2WeightedPoolFactoryV4",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/tasks/20230320-weighted-pool-v4/artifact/WeightedPoolFactory.json",
        )?
        .github(
            "BalancerV2LiquidityBootstrappingPoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20210721-liquidity-bootstrapping-pool/artifact/LiquidityBootstrappingPoolFactory.json",
        )?
        .github(
            "BalancerV2StablePoolFactoryV2",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20220609-stable-pool-v2/artifact/StablePoolFactory.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20220906-composable-stable-pool/artifact/ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactoryV3",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20230206-composable-stable-pool-v3/artifact/ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactoryV4",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20230320-composable-stable-pool-v4/artifact/ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactoryV5",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20230711-composable-stable-pool-v5/artifact/ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactoryV6",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/tasks/20240223-composable-stable-pool-v6/artifact/ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2NoProtocolFeeLiquidityBootstrappingPoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/tasks/20211202-no-protocol-fee-lbp/artifact/NoProtocolFeeLiquidityBootstrappingPoolFactory.json",
        )?
        .github(
            "BalancerV2GyroECLPPoolFactory",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/1/0xA87217d855EDad15d63255DB176B810b0325BDa2.json",
        )?
        .github(
            "BalancerV2Gyro2CLPPoolFactory",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/137/0xfa08Eb3A781532f47b1dd811A6cA326842EA0CB5.json",
        )?
        .github(
            "BalancerV2Gyro3CLPPoolFactory",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/137/0x977e8A3917DFFe582aFbb2EE5a6be3E9fa043079.json",
        )?
        .github(
            "BalancerV2LiquidityBootstrappingPool",
            "balancer-labs/balancer-v2-monorepo/7a643349a5ef4511234b19a33e3f18d30770cb66/pkg/deployments/tasks/20210721-liquidity-bootstrapping-pool/abi/LiquidityBootstrappingPool.json",
        )?
        .github(
            "BalancerV2WeightedPool",
            "balancer-labs/balancer-v2-monorepo/a3b570a2aa655d4c4941a67e3db6a06fbd72ef09/pkg/deployments/extra-abis/WeightedPool.json",
        )?
        .github(
            "BalancerV2StablePool",
            "balancer-labs/balancer-subgraph-v2/2b97edd5e65aed06718ce64a69111ccdabccf048/abis/StablePool.json",
        )?
        .github(
            "BalancerV2ComposableStablePool",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v2/deprecated/20230206-composable-stable-pool-v3/artifact/ComposableStablePool.json",
        )?
        .github(
            "BalancerV2GyroECLPPool",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/1/0x767CB26E38beBddfd857d4Ad426d8ff5Bc0fAc2f.json",
        )?
        .github(
            "BalancerV2Gyro2CLPPool",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/137/0xeD6850f0400c111e49Ee3F4bEa44d19b438B4611.json",
        )?
        .github(
            "BalancerV2Gyro3CLPPool",
            "gyrostable/gyro-pools/8e83ed6946e77cb8f438d1ce3ae615f490c27f4e/build/deployments/137/0xB211043347Cb9f2dFD8b981996820E2886B0eF18.json",
        )?;

    // Balancer V2 contracts - ABI Only
    vendor
        .abi_only()
        .manual(
            "BalancerV2BasePool",
            "Balancer does not publish ABIs for base contracts",
        )
        .manual(
            "BalancerV2BasePoolFactory",
            "Balancer does not publish ABIs for base contracts",
        );
    
    // Balancer V3 contracts - Full
    vendor
        .full()
        // We use the VaultExtension ABI for the Vault contract to get all the methods due to the proxy pattern
        .github(
            "BalancerV3Vault",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/tasks/20241204-v3-vault/artifact/VaultExtension.json",
        )?
        .github(
            "BalancerV3BatchRouter",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/tasks/20241205-v3-batch-router/artifact/BatchRouter.json",
        )?
        .github(
            "BalancerV3WeightedPoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/tasks/20241205-v3-weighted-pool/artifact/WeightedPoolFactory.json",
        )?
        .github(
            "BalancerV3StablePoolFactory",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/deprecated/20241205-v3-stable-pool/artifact/StablePoolFactory.json",
        )?
        .github(
            "BalancerV3StablePoolFactoryV2",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/tasks/20250324-v3-stable-pool-v2/artifact/StablePoolFactory.json",
        )?
        .github(
            "BalancerV3GyroECLPPoolFactory",
            "balancer/balancer-deployments/7a16f4463fb33d5f8c63118b6427b73fdb727cfc/v3/tasks/20250124-v3-gyro-eclp/artifact/GyroECLPPoolFactory.json",
        )?
        .github(
            "BalancerV3Gyro2CLPPoolFactory",
            "balancer/balancer-deployments/18839b81d136315c587011c135958b49272547ab/v3/tasks/20250120-v3-gyro-2clp/artifact/Gyro2CLPPoolFactory.json",
        )?
        .github(
            "BalancerV3ReClammPoolFactoryV2",
            "balancer/balancer-deployments/18839b81d136315c587011c135958b49272547ab/v3/tasks/20250702-v3-reclamm-pool-v2/artifact/ReClammPoolFactory.json",
        )?
        .github(
            "BalancerV3StableSurgePoolFactory",
            "balancer/balancer-deployments/6c09a56033f2072cfac8823105df24e08dac812c/v3/deprecated/20250121-v3-stable-surge/artifact/StableSurgePoolFactory.json",
        )?
        .github(
            "BalancerV3StableSurgePoolFactoryV2",
            "balancer/balancer-deployments/6c09a56033f2072cfac8823105df24e08dac812c/v3/tasks/20250404-v3-stable-surge-pool-factory-v2/artifact/StableSurgePoolFactory.json",
        )?
        .github(
            "BalancerV3StableSurgeHook",
            "balancer/balancer-deployments/6c09a56033f2072cfac8823105df24e08dac812c/v3/tasks/20250403-v3-stable-surge-hook-v2/artifact/StableSurgeHook.json",
        )?
        .manual(
            "BalancerV3QuantAMMWeightedPoolFactory",
            "Manually vendored ABI for Quant AMM Weighted Pool Factory contract",
        );

    // Balancer V3 contracts - ABI Only
    vendor
        .abi_only()
        .github(
            "BalancerV3WeightedPool",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/tasks/20241205-v3-weighted-pool/artifact/WeightedPool.json",
        )?
        .github(
            "BalancerV3StablePool",
            "balancer/balancer-deployments/48cb2fcbf17769f09c4ed905613b04db7707cfde/v3/deprecated/20241205-v3-stable-pool/artifact/StablePool.json",
        )?
        .github(
            "BalancerV3StableSurgePool",
            "balancer/balancer-deployments/6c09a56033f2072cfac8823105df24e08dac812c/v3/deprecated/20250121-v3-stable-surge/artifact/StablePool.json",
        )?
        .github(
            "BalancerV3GyroECLPPool",
            "balancer/balancer-deployments/7a16f4463fb33d5f8c63118b6427b73fdb727cfc/v3/tasks/20250124-v3-gyro-eclp/artifact/GyroECLPPool.json",
        )?
        .github(
            "BalancerV3Gyro2CLPPool",
            "balancer/balancer-deployments/18839b81d136315c587011c135958b49272547ab/v3/tasks/20250120-v3-gyro-2clp/artifact/Gyro2CLPPool.json",
        )?
        .github(
            "BalancerV3ReClammPool",
            "balancer/balancer-deployments/18839b81d136315c587011c135958b49272547ab/v3/tasks/20250702-v3-reclamm-pool-v2/artifact/ReClammPool.json",
        )?
        .manual(
            "BalancerV3QuantAMMWeightedPool",
            "Manually vendored ABI for Quant AMM Weighted Pool contract",
        );

    // CowSwap contracts - Full
    vendor
        .full()
        .npm(
            "CowProtocolToken",
            "@cowprotocol/token@1.1.0/build/artifacts/src/contracts/CowProtocolToken.sol/CowProtocolToken.json",
        )?
        .github(
            "CoWSwapEthFlow",
            &format!("cowprotocol/ethflowcontract/{ETHFLOW_VERSION}-artifacts/hardhat-artifacts/src/CoWSwapEthFlow.sol/CoWSwapEthFlow.json"),
        )?
        .npm(
            "ERC20Mintable",
            "@openzeppelin/contracts@2.5.0/build/contracts/ERC20Mintable.json",
        )?
        .npm(
            "GPv2AllowListAuthentication",
            // We use `_Implementation` because the use of a proxy contract makes
            // deploying  for the e2e tests more cumbersome.
            "@cowprotocol/contracts@1.1.2/deployments/mainnet/GPv2AllowListAuthentication_Implementation.json",
        )?
        .npm(
            "GPv2Settlement",
            "@cowprotocol/contracts@1.1.2/deployments/mainnet/GPv2Settlement.json",
        )?
        .npm(
            "GnosisSafe",
            "@gnosis.pm/safe-contracts@1.3.0/build/artifacts/contracts/GnosisSafe.sol/GnosisSafe.json",
        )?
        .npm(   
            "GnosisSafeCompatibilityFallbackHandler",
            "@gnosis.pm/safe-contracts@1.3.0/build/artifacts/contracts/handler/CompatibilityFallbackHandler.sol/CompatibilityFallbackHandler.json",
        )?
        .npm(
            "GnosisSafeProxy",
            "@gnosis.pm/safe-contracts@1.3.0/build/artifacts/contracts/proxies/GnosisSafeProxy.sol/GnosisSafeProxy.json",
        )?
        .npm(
            "GnosisSafeProxyFactory",
            "@gnosis.pm/safe-contracts@1.3.0/build/artifacts/contracts/proxies/GnosisSafeProxyFactory.sol/GnosisSafeProxyFactory.json",
        )?
        .manual(
            "HooksTrampoline",
            "Manually vendored ABI and bytecode for hooks trampoline contract",
        )
        .manual(
            "LiquoriceSettlement",
            "Liquorice does not publish its code",
        )
        .npm(
            "UniswapV2Factory",
            "@uniswap/v2-core@1.0.1/build/UniswapV2Factory.json",
        )?
        .npm(
            "UniswapV2Router02",
            "@uniswap/v2-periphery@1.1.0-beta.0/build/UniswapV2Router02.json",
        )?
        .npm(
            "WETH9",
            "canonical-weth@1.4.0/build/contracts/WETH9.json",
        )?
        .manual(
            "WETH9",
            "Manually vendored ABI and bytecode for WETH9 contract",
        );

    // CowSwap contracts - ABI Only
    vendor
        .abi_only()
        .github(
            "BalancerV2ComposableStablePool",
            "balancer/balancer-deployments/7211ed9f209d767f8ff411bb57262daf83ecb439/\
             tasks/deprecated/20230206-composable-stable-pool-v3/artifact/\
             ComposableStablePool.json",
        )?
        .github(
            "BalancerQueries",
            "balancer/balancer-deployments/80505f7061ff668dc55173344564fa1ddd83b726/\
            v2/tasks/20220721-balancer-queries/artifact/BalancerQueries.json",
        )?
        .github(
            "BalancerV2ComposableStablePoolFactory",
            "balancer/balancer-deployments/7211ed9f209d767f8ff411bb57262daf83ecb439/\
             tasks/deprecated/20230206-composable-stable-pool-v3/artifact/\
             ComposableStablePoolFactory.json",
        )?
        .github(
            "BalancerV2LiquidityBootstrappingPool",
            "balancer-labs/balancer-v2-monorepo/7a643349a5ef4511234b19a33e3f18d30770cb66/pkg/\
             deployments/tasks/20210721-liquidity-bootstrapping-pool/abi/\
             LiquidityBootstrappingPool.json",
        )?
        .github(
            "BalancerV2LiquidityBootstrappingPoolFactory",
            "balancer-labs/balancer-v2-monorepo/7a643349a5ef4511234b19a33e3f18d30770cb66/pkg/\
             deployments/tasks/20210721-liquidity-bootstrapping-pool/abi/\
             LiquidityBootstrappingPoolFactory.json",
        )?
        .github(
            "BalancerV2WeightedPool",
            "balancer-labs/balancer-v2-monorepo/a3b570a2aa655d4c4941a67e3db6a06fbd72ef09/pkg/\
             deployments/extra-abis/WeightedPool.json",
        )?
        .github(
            "BalancerV2StablePool",
            "balancer-labs/balancer-subgraph-v2/2b97edd5e65aed06718ce64a69111ccdabccf048/abis/\
             StablePool.json",
        )?
        .github(
            "BalancerV2StablePoolFactoryV2",
            "balancer-labs/balancer-v2-monorepo/903d34e491a5e9c5d59dabf512c7addf1ccf9bbd/pkg/\
             deployments/tasks/20220609-stable-pool-v2/abi/StablePoolFactory.json",
        )?
        .github(
            "CoWSwapOnchainOrders",
            &format!("cowprotocol/ethflowcontract/{ETHFLOW_VERSION}-artifacts/hardhat-artifacts/src/mixins/CoWSwapOnchainOrders.sol/CoWSwapOnchainOrders.json"),
        )?
        .npm(
            "ERC20",
            "@openzeppelin/contracts@3.3.0/build/contracts/ERC20.json",
        )?
        .manual(
            "ERC1271SignatureValidator",
            "Manually vendored ABI for ERC-1271 signature validation",
        )
        .npm(
            "IUniswapLikePair",
            "@uniswap/v2-periphery@1.1.0-beta.0/build/IUniswapV2Pair.json",
        )?
        .npm(
            "IUniswapLikeRouter",
            "@uniswap/v2-periphery@1.1.0-beta.0/build/IUniswapV2Router02.json",
        )?
        .npm(
            "IUniswapV3Factory",
            "@uniswap/v3-core@1.0.0/artifacts/contracts/interfaces/IUniswapV3Factory.sol/IUniswapV3Factory.json",
        )?
        .github(
            "IZeroEx",
            "0xProject/protocol/c1177416f50c2465ee030dacc14ff996eebd4e74/packages/contract-artifacts/artifacts/IZeroEx.json",
        )?
        .github(
            "ISwaprPair",
            "levelkdev/dxswap-core/3511bab996096f9c9c9bc3af0d94222650fd1e40/build/IDXswapPair.json",
        )?
        .manual(
            "ChainalysisOracle",
            "Chainalysis does not publish its code",
        );
    
    Ok(())
}

struct Vendor {
    artifacts: PathBuf,
}

impl Vendor {
    fn try_new() -> Result<Self> {
        let artifacts = paths::contract_artifacts_dir();
        tracing::info!("vendoring contract artifacts to '{}'", artifacts.display());
        fs::create_dir_all(&artifacts)?;
        Ok(Self { artifacts })
    }

    /// Creates a context for vendoring "full" contract data, including bytecode
    /// used for deploying the contract for end-to-end test.
    fn full(&self) -> VendorContext<'_> {
        VendorContext {
            artifacts: &self.artifacts,
            properties: &[
                ("abi", "abi,compilerOutput.abi"),
                ("devdoc", "devdoc,compilerOutput.devdoc"),
                ("userdoc", "userdoc"),
                ("bytecode", "bytecode"),
            ],
        }
    }

    /// Creates a context for vendoring only the contract ABI for generating
    /// bindings. This is preferred over [`Vendor::full`] for contracts that do
    /// not need to be deployed for tests, or get created by alternative means
    /// (e.g. `UniswapV2Pair` contracts don't require bytecode as they get
    /// created by `UniswapV2Factory` instances on-chain).
    fn abi_only(&self) -> VendorContext<'_> {
        VendorContext {
            artifacts: &self.artifacts,
            properties: &[
                ("abi", "abi,compilerOutput.abi"),
                ("devdoc", "devdoc,compilerOutput.devdoc"),
                ("userdoc", "userdoc"),
            ],
        }
    }
}

struct VendorContext<'a> {
    artifacts: &'a Path,
    properties: &'a [(&'a str, &'a str)],
}

impl VendorContext<'_> {
    fn npm(&self, name: &str, path: &str) -> Result<&Self> {
        self.vendor_source(name, Source::npm(path))
    }

    fn github(&self, name: &str, path: &str) -> Result<&Self> {
        self.vendor_source(
            name,
            Source::http(&format!("https://raw.githubusercontent.com/{path}"))?,
        )
    }

    fn manual(&self, name: &str, reason: &str) -> &Self {
        // We just keep these here to document that they are manually generated
        // and not pulled from some source.
        tracing::info!("skipping {}: {}", name, reason);
        self
    }

    fn retrieve_value_from_path<'a>(source: &'a Value, path: &'a str) -> Value {
        let mut current_value: &Value = source;
        for property in path.split('.') {
            current_value = &current_value[property];
        }
        current_value.clone()
    }

    fn vendor_source(&self, name: &str, source: Source) -> Result<&Self> {
        tracing::info!("retrieving {:?}", source);
        let artifact_json = source.artifact_json()?;

        tracing::debug!("pruning artifact JSON");
        let pruned_artifact_json = {
            let json = serde_json::from_str::<Value>(&artifact_json)?;
            let mut pruned = Map::new();
            for (property, paths) in self.properties {
                if let Some(value) = paths
                    .split(',')
                    .map(|path| Self::retrieve_value_from_path(&json, path))
                    .find(|value| !value.is_null())
                {
                    pruned.insert(property.to_string(), value);
                }
            }
            serde_json::to_string(&pruned)?
        };

        let path = self.artifacts.join(name).with_extension("json");
        tracing::debug!("saving artifact to {}", path.display());
        fs::write(path, pruned_artifact_json)?;

        Ok(self)
    }
}
