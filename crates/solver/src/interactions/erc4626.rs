use {
    alloy::{primitives::U256, sol_types::SolCall},
    contracts::alloy::IERC4626,
    ethrpc::alloy::conversions::IntoAlloy,
    shared::interaction::{EncodedInteraction, Interaction},
};

#[derive(Clone, Debug)]
pub struct MintExactSharesInteraction {
    pub vault: IERC4626::Instance,
    pub shares_out: primitive_types::U256,
    pub receiver: primitive_types::H160,
}

impl Interaction for MintExactSharesInteraction {
    fn encode(&self) -> EncodedInteraction {
        let calldata = IERC4626::IERC4626::mintCall {
            shares: self.shares_out.into_alloy(),
            receiver: self.receiver.into_alloy(),
        }
        .abi_encode();
        (
            *self.vault.address(),
            U256::ZERO,
            alloy::primitives::Bytes::from(calldata),
        )
    }
}

#[derive(Clone, Debug)]
pub struct WithdrawExactAssetsInteraction {
    pub vault: IERC4626::Instance,
    pub assets_out: primitive_types::U256,
    pub receiver: primitive_types::H160,
    pub owner: primitive_types::H160,
}

impl Interaction for WithdrawExactAssetsInteraction {
    fn encode(&self) -> EncodedInteraction {
        let calldata = IERC4626::IERC4626::withdrawCall {
            assets: self.assets_out.into_alloy(),
            receiver: self.receiver.into_alloy(),
            owner: self.owner.into_alloy(),
        }
        .abi_encode();
        (
            *self.vault.address(),
            U256::ZERO,
            alloy::primitives::Bytes::from(calldata),
        )
    }
}

#[cfg(test)]
mod tests {
    use {super::*, hex_literal::hex, primitive_types::H160};

    fn dummy_vault(addr: alloy::primitives::Address) -> IERC4626::Instance {
        // Create a dummy provider for testing - only address matters for encoding
        let web3 = ethrpc::mock::web3();
        IERC4626::Instance::new(addr, web3.alloy)
    }

    #[test]
    fn encode_mint_exact_shares() {
        let vault_addr = alloy::primitives::Address::repeat_byte(0x11);
        let vault = dummy_vault(vault_addr);
        let interaction = MintExactSharesInteraction {
            vault,
            shares_out: primitive_types::U256::from(123u64),
            receiver: H160([0x22; 20]),
        };
        let (target, value, calldata) = interaction.encode();
        assert_eq!(target, vault_addr);
        assert_eq!(value, alloy::primitives::U256::ZERO);
        // selector 0x94bf804d (mint(uint256,address))
        assert_eq!(&calldata[0..4], &hex!("94bf804d"));
    }

    #[test]
    fn encode_withdraw_exact_assets() {
        let vault_addr = alloy::primitives::Address::repeat_byte(0x33);
        let vault = dummy_vault(vault_addr);
        let interaction = WithdrawExactAssetsInteraction {
            vault,
            assets_out: primitive_types::U256::from(456u64),
            receiver: H160([0x44; 20]),
            owner: H160([0x55; 20]),
        };
        let (target, value, calldata) = interaction.encode();
        assert_eq!(target, vault_addr);
        assert_eq!(value, alloy::primitives::U256::ZERO);
        // selector 0xb460af94 (withdraw(uint256,address,address))
        assert_eq!(&calldata[0..4], &hex!("b460af94"));
    }
}
