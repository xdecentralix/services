use {
    contracts::IERC4626,
    ethcontract::Bytes,
    shared::interaction::{EncodedInteraction, Interaction},
};

#[derive(Clone, Debug)]
pub struct MintExactSharesInteraction {
    pub vault: IERC4626,
    pub shares_out: primitive_types::U256,
    pub receiver: primitive_types::H160,
}

impl Interaction for MintExactSharesInteraction {
    fn encode(&self) -> EncodedInteraction {
        let method = self.vault.mint(self.shares_out, self.receiver);
        let calldata = method.tx.data.expect("no calldata").0;
        (self.vault.address(), 0.into(), Bytes(calldata))
    }
}

#[derive(Clone, Debug)]
pub struct WithdrawExactAssetsInteraction {
    pub vault: IERC4626,
    pub assets_out: primitive_types::U256,
    pub receiver: primitive_types::H160,
    pub owner: primitive_types::H160,
}

impl Interaction for WithdrawExactAssetsInteraction {
    fn encode(&self) -> EncodedInteraction {
        let method = self
            .vault
            .withdraw(self.assets_out, self.receiver, self.owner);
        let calldata = method.tx.data.expect("no calldata").0;
        (self.vault.address(), 0.into(), Bytes(calldata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::dummy_contract;
    use hex_literal::hex;
    use primitive_types::{H160, U256};

    #[test]
    fn encode_mint_exact_shares() {
        let vault = dummy_contract!(IERC4626, H160([0x11; 20]));
        let interaction = MintExactSharesInteraction {
            vault: vault.clone(),
            shares_out: U256::from(123u64),
            receiver: H160([0x22; 20]),
        };
        let (target, value, calldata) = interaction.encode();
        assert_eq!(target, vault.address());
        assert_eq!(value, 0.into());
        // selector 0x94bf804d (mint(uint256,address))
        assert_eq!(&calldata.0[0..4], &hex!("94bf804d"));
    }

    #[test]
    fn encode_withdraw_exact_assets() {
        let vault = dummy_contract!(IERC4626, H160([0x33; 20]));
        let interaction = WithdrawExactAssetsInteraction {
            vault: vault.clone(),
            assets_out: U256::from(456u64),
            receiver: H160([0x44; 20]),
            owner: H160([0x55; 20]),
        };
        let (target, value, calldata) = interaction.encode();
        assert_eq!(target, vault.address());
        assert_eq!(value, 0.into());
        // selector 0xb460af94 (withdraw(uint256,address,address))
        assert_eq!(&calldata.0[0..4], &hex!("b460af94"));
    }
}


