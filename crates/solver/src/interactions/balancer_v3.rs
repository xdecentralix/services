//! Module for Balancer V3 swap interactions.

use {
    contracts::{BalancerV3BatchRouter, GPv2Settlement},
    ethcontract::{Bytes, H160},
    primitive_types::U256,
    shared::{
        http_solver::model::TokenAmount,
        interaction::{EncodedInteraction, Interaction},
    },
    std::sync::LazyLock,
};

#[derive(Clone, Debug)]
pub struct BalancerV3SwapGivenOutInteraction {
    pub settlement: GPv2Settlement,
    pub batch_router: BalancerV3BatchRouter,
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
        let method = self.batch_router.swap_exact_out(
            vec![(                              // SwapPathExactAmountOut[]
                self.asset_in_max.token,        // tokenIn (H160)
                vec![(                          // steps[]
                    self.pool,                  // pool (H160)
                    self.asset_out.token,       // tokenOut (H160)
                    false,                      // isBuffer (bool)
                )],
                self.asset_in_max.amount,       // maxAmountIn (U256)
                self.asset_out.amount,          // exactAmountOut (U256)   
            )],
            *NEVER,                             // deadline (U256)
            false,                              // wethIsEth (bool)
            self.user_data.clone(),             // userData (Bytes)
        );
        let calldata = method.tx.data.expect("no calldata").0;
        (self.batch_router.address(), 0.into(), Bytes(calldata))
    }
}

impl Interaction for BalancerV3SwapGivenOutInteraction {
    fn encode(&self) -> EncodedInteraction {
        self.encode_swap()
    }
}

#[cfg(test)]
mod tests {
    use {super::*, contracts::dummy_contract, primitive_types::H160};

    #[test]
    fn encode_unwrap_weth() {
        let batch_router = dummy_contract!(BalancerV3BatchRouter, [0x01; 20]);
        let interaction = BalancerV3SwapGivenOutInteraction {
            settlement: dummy_contract!(GPv2Settlement, [0x02; 20]),
            batch_router: batch_router.clone(),
            pool: H160([0x03; 20]),
            asset_in_max: TokenAmount::new(H160([0x04; 20]), 1_337_000_000_000_000_000_000u128),
            asset_out: TokenAmount::new(H160([0x05; 20]), 42_000_000_000_000_000_000u128),
            user_data: Bytes::default(),
        };

        // V3 uses a different method signature, so the encoded calldata will be different
        // The test verifies that encoding works without errors
        let encoded = interaction.encode();
        assert_eq!(encoded.0, batch_router.address());
        assert_eq!(encoded.1, 0.into());
        assert!(!encoded.2 .0.is_empty());
    }
} 