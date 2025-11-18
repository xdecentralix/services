//! Module for Balancer V3 swap interactions.

use {
    alloy::primitives::U256,
    contracts::alloy::{
        BalancerV3BatchRouter::{
            self,
            IBatchRouter::{SwapPathExactAmountOut, SwapPathStep},
        },
        GPv2Settlement,
    },
    ethcontract::{Bytes, H160},
    ethrpc::alloy::conversions::{IntoAlloy, IntoLegacy},
    shared::{
        http_solver::model::TokenAmount,
        interaction::{EncodedInteraction, Interaction},
    },
    std::sync::LazyLock,
};

#[derive(Clone, Debug)]
pub struct BalancerV3SwapGivenOutInteraction {
    pub settlement: GPv2Settlement::Instance,
    pub batch_router: BalancerV3BatchRouter::Instance,
    pub pool: H160,
    pub asset_in_max: TokenAmount,
    pub asset_out: TokenAmount,
    pub user_data: Bytes<Vec<u8>>,
}

/// An impossibly distant future timestamp. Note that we use `0x80000...00`
/// as the value so that it is mostly 0's to save small amounts of gas on
/// calldata.
pub static NEVER: LazyLock<U256> = LazyLock::new(|| U256::from(1) << 255);

impl BalancerV3SwapGivenOutInteraction {
    pub fn encode_swap(&self) -> EncodedInteraction {
        let swap_path = SwapPathExactAmountOut {
            tokenIn: self.asset_in_max.token.into_alloy(),
            steps: vec![SwapPathStep {
                pool: self.pool.into_alloy(),
                tokenOut: self.asset_out.token.into_alloy(),
                isBuffer: false,
            }]
            .into(),
            maxAmountIn: self.asset_in_max.amount.into_alloy(),
            exactAmountOut: self.asset_out.amount.into_alloy(),
        };
        let method = self
            .batch_router
            .swapExactOut(
                vec![swap_path].into(),
                *NEVER,
                false,
                self.user_data.clone().into_alloy(),
            )
            .calldata()
            .clone();

        (
            self.batch_router.address().into_legacy(),
            0.into(),
            Bytes(method.to_vec()),
        )
    }
}

impl Interaction for BalancerV3SwapGivenOutInteraction {
    fn encode(&self) -> EncodedInteraction {
        self.encode_swap()
    }
}

#[cfg(test)]
mod tests {
    use {super::*, primitive_types::H160};

    #[test]
    fn encode_unwrap_weth() {
        let batch_router =
            BalancerV3BatchRouter::Instance::new([0x01; 20].into(), ethrpc::mock::web3().alloy);
        let settlement =
            GPv2Settlement::Instance::new([0x02; 20].into(), ethrpc::mock::web3().alloy);
        let interaction = BalancerV3SwapGivenOutInteraction {
            settlement,
            batch_router: batch_router.clone(),
            pool: H160([0x03; 20]),
            asset_in_max: TokenAmount::new(H160([0x04; 20]), 1_337_000_000_000_000_000_000u128),
            asset_out: TokenAmount::new(H160([0x05; 20]), 42_000_000_000_000_000_000u128),
            user_data: Bytes::default(),
        };

        // V3 uses a different method signature, so the encoded calldata will be
        // different The test verifies that encoding works without errors
        let encoded = interaction.encode();
        assert_eq!(encoded.0, batch_router.address().into_legacy());
        assert_eq!(encoded.1, 0.into());
        assert!(!encoded.2.0.is_empty());
    }
}
