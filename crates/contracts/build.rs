use {
    ethcontract::{
        Address,
        common::{DeploymentInformation, contract::Network},
    },
    ethcontract_generate::{ContractBuilder, loaders::TruffleLoader},
    std::{env, path::Path},
};

#[path = "src/paths.rs"]
mod paths;

const MAINNET: &str = "1";
const OPTIMISM: &str = "10";
const BNB: &str = "56";
const GNOSIS: &str = "100";
const SEPOLIA: &str = "11155111";
const ARBITRUM_ONE: &str = "42161";
const BASE: &str = "8453";
const POLYGON: &str = "137";
const AVALANCHE: &str = "43114";
const LENS: &str = "232";

fn main() {
    // NOTE: This is a workaround for `rerun-if-changed` directives for
    // non-existent files cause the crate's build unit to get flagged for a
    // rebuild if any files in the workspace change.
    //
    // See:
    // - https://github.com/rust-lang/cargo/issues/6003
    // - https://doc.rust-lang.org/cargo/reference/build-scripts.html#cargorerun-if-changedpath
    println!("cargo:rerun-if-changed=build.rs");

    generate_contract_with_config("BalancerV2Authorizer", |builder| {
        builder.contract_mod_override("balancer_v2_authorizer")
    });
    generate_contract_with_config("BalancerV2Vault", |builder| {
        builder
            .contract_mod_override("balancer_v2_vault")
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://etherscan.io/tx/0x28c44bb10d469cbd42accf97bd00b73eabbace138e9d44593e851231fbed1cb7>
                    deployment_information: Some(DeploymentInformation::BlockNumber(12272146)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://gnosisscan.io/tx/0x21947751661e1b9197492f22779af1f5175b71dc7057869e5a8593141d40edf1>
                    deployment_information: Some(DeploymentInformation::BlockNumber(24821598)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://sepolia.etherscan.io/tx/0xb22509c6725dd69a975ecb96a0c594901eeee6a279cc66d9d5191022a7039ee6>
                    deployment_information: Some(DeploymentInformation::BlockNumber(3418831)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://arbiscan.io/tx/0xe2c3826bd7b15ef8d338038769fe6140a44f1957a36b0f27ab321ab6c68d5a8e>
                    deployment_information: Some(DeploymentInformation::BlockNumber(222832)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://basescan.org/tx/0x0dc2e3d436424f2f038774805116896d31828c0bf3795a6901337bdec4e0dff6>
                    deployment_information: Some(DeploymentInformation::BlockNumber(1196036)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://snowscan.xyz/tx/0xc49af0372feb032e0edbba6988410304566b1fd65546c01ced620ac3c934120f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(26386141)),
                },
            )
            .add_network(
                BNB,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://bscscan.com/tx/0x1de8caa6c54ff9a25600e26d80865d84c9cc4d33c2b98611240529ee7de5cd74>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22691002)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://optimistic.etherscan.io/tx/0xa03cb990595df9eed6c5db17a09468cab534aed5f5589a06c0bb3d19dd2f7ce9>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7003431)),
                },
            )
            .add_network(
                POLYGON,
                Network {
                    address: addr("0xBA12222222228d8Ba445958a75a0704d566BF2C8"),
                    // <https://polygonscan.com/tx/0x66f275a2ed102a5b679c0894ced62c4ebcb2a65336d086a916eb83bd1fe5c8d2>
                    deployment_information: Some(DeploymentInformation::BlockNumber(15832990)),
                },
            )
        // Not available on Lens
    });

    // Balancer V3 contracts
    generate_contract_with_config("BalancerV3Vault", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://etherscan.io/tx/0x49a4986a672bcc20eecf99a3603f0099b19ab663eebe5dd5fe04808c380147b4>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21332121)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://gnosisscan.io/tx/0x754f9db9925c52591e5d9d6233979fefb19a60aa3768f5b54daf8ddadb08f23a>
                    deployment_information: Some(DeploymentInformation::BlockNumber(37360338)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://sepolia.etherscan.io/tx/0xe9ab355e0f5987453c48b3fe64f7c63ae4ba6dc5a85d1e43fb3a066dffe16a81>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7212247)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://arbiscan.io/tx/0x8fbebf41ac79cd928ec75867c69afa9c2635b44215b21e2891e650f85f3c4f27>
                    deployment_information: Some(DeploymentInformation::BlockNumber(297810187)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://basescan.org/tx/0xd11401d622a1b097c5b3822bd75c765c63fbe59fa40fe5e32466067ff4e6ded2>
                    deployment_information: Some(DeploymentInformation::BlockNumber(25343854)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://snowscan.xyz/tx/0x83f1f156e2d09961087e3a52464ae7432e250954e55756d4728040ff27a63c9c>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59955604)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0xbA1333333333a1BA1108E8412f11850A5C319bA9"),
                    // <https://optimistic.etherscan.io/tx/0x6e0786a4eec8586f5cb100ba610f0e50f4dfbc173b1fad2a1153cfe3e754791d>
                    deployment_information: Some(DeploymentInformation::BlockNumber(133969439)),
                },
            )
    });
    generate_contract_with_config("BalancerV3BatchRouter", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x136f1EFcC3f8f88516B9E94110D56FDBfB1778d1"),
                    // <https://etherscan.io/tx/0x41cb8619fb92dd532eb09b0e81fd4ce1c6006a10924893f02909e36a317777f3>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21339510)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xe2fa4e1d17725e72dcdAfe943Ecf45dF4B9E285b"),
                    // <https://gnosisscan.io/tx/0xeafddbace9f445266f851ef1d92928e3d01a4622a1a6780b41ac52d5872f12ab>
                    deployment_information: Some(DeploymentInformation::BlockNumber(37377506)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xC85b652685567C1B074e8c0D4389f83a2E458b1C"),
                    // <https://sepolia.etherscan.io/tx/0x95ed8e1aaaa7bdc5881f3c8fc5a4914a66639bee52987c3a1ea88545083b0681>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7219301)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0xaD89051bEd8d96f045E8912aE1672c6C0bF8a85E"),
                    // <https://arbiscan.io/tx/0xa7968c6bc0775208ffece789c6e5d09b0eea5f2c3ed2806e9bd94fb0b978ff0f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(297828544)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x85a80afee867aDf27B50BdB7b76DA70f1E853062"),
                    // <https://basescan.org/tx/0x47b81146714630ce50445bfa28872a36973acedf785317ca423498810ec8e76c>
                    deployment_information: Some(DeploymentInformation::BlockNumber(25347205)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xc9b36096f5201ea332Db35d6D195774ea0D5988f"),
                    // <https://snowscan.xyz/tx/0x3bfaba7135ee2d67d98f20ee1aa4c8b7e81e47be64223376f3086bab429ac806>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59965747)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0xaD89051bEd8d96f045E8912aE1672c6C0bF8a85E"),
                    // <https://optimistic.etherscan.io/tx/0xf370aab0d652f3e0f7c34e1a53e1afd98e86c487138300b0939d4e54b0088b67>
                    deployment_information: Some(DeploymentInformation::BlockNumber(133969588)),
                },
            )
        // Not available on Lens
    });
    generate_contract_with_config("BalancerV3WeightedPoolFactory", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0xD961E30156C2E0D0d925A0De45f931CB7815e970"),
                    // <https://arbiscan.io/tx/0x3ffc0d75e73499568fa2de95c4923256333653afef2d6dd6f51596b1835c81ad>
                    deployment_information: Some(DeploymentInformation::BlockNumber(297830075)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xD961E30156C2E0D0d925A0De45f931CB7815e970"),
                    // <https://snowscan.xyz/tx/0xdd6735ab7addc99d9a3132f3dca03c109c8c1cb46aff97e75655a5d0e37e515a>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59965815)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x5cF4928a3205728bd12830E1840F7DB85c62a4B9"),
                    // <https://basescan.org/tx/0xa3d57290120458e4a1e011d4791c79dff3072bc23ea52e6b9df615019c3cf341>
                    deployment_information: Some(DeploymentInformation::BlockNumber(25347415)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xEB1eeaBF0126d813589C3D2CfeFFE410D9aE3863"),
                    // <https://gnosisscan.io/tx/0x04965cda30a501e074b983c40c5ff83d70401597da929e937e39d60022f4f0d9>
                    deployment_information: Some(DeploymentInformation::BlockNumber(37371691)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x201efd508c8DfE9DE1a13c2452863A78CB2a86Cc"),
                    // <https://etherscan.io/tx/0x1e14baaeb10fc3a6b689e77ec34e8c5e8e21853f6e23257459dd99c35b6ff06b>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21336937)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x0f08eEf2C785AA5e7539684aF04755dEC1347b7c"),
                    // <https://optimistic.etherscan.io/tx/0x68adbde1153361bc5cc11d68e950169e12edb9d6d747856063da9244477cfb07>
                    deployment_information: Some(DeploymentInformation::BlockNumber(133969639)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x7532d5a3bE916e4a4D900240F49F0BABd4FD855C"),
                    // <https://sepolia.etherscan.io/tx/0xe42c9cdc05ab3de2b8698ed32e56dce0f85c1017099aa965784d8023fb29d012>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7216947)),
                },
            )
    });
    generate_contract_with_config("BalancerV3StablePoolFactory", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0xEB1eeaBF0126d813589C3D2CfeFFE410D9aE3863"),
                    // <https://arbiscan.io/tx/0xe53025dfdda3dc70ef148b3b14db804161f27fcda5a9805188b56ff9cc29db41>
                    deployment_information: Some(DeploymentInformation::BlockNumber(297829373)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0xb9F8AB3ED3F3aCBa64Bc6cd2DcA74B7F38fD7B88"),
                    // <https://basescan.org/tx/0xc8721c34e82df9b8ce40cb2451b05cdf10b91b602ef9e0f473ca2af4da733997>
                    deployment_information: Some(DeploymentInformation::BlockNumber(25347318)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x22625eEDd92c81a219A83e1dc48f88d54786B017"),
                    // <https://gnosisscan.io/tx/0xbd157de3b2e45017b96a93474051c6f390f4c5d46a178a8a2e96c7b68ca85873>
                    deployment_information: Some(DeploymentInformation::BlockNumber(37371860)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xB9d01CA61b9C181dA1051bFDd28e1097e920AB14"),
                    // <https://etherscan.io/tx/0x2794463090a850910415b88df0f756e01e0838c8782e83a89389992c17469513>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21337005)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xd67F485C07D258B3e93835a3799d862ffcB55923"),
                    // <https://sepolia.etherscan.io/tx/0x48d803b01baf630543481ca6eefca5dc269d8670cf44afd08dcba3792a48710f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7217020)),
                },
            )
    });
    generate_contract_with_config("BalancerV3StablePoolFactoryV2", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x44d33798dddCdAbc93Fe6a40C80588033Dc502d3"),
                    // <https://arbiscan.io/tx/0x39b85ee778313036afde824463fdb74d2dea60621a4e17744d962ba34f80ad4b>
                    deployment_information: Some(DeploymentInformation::BlockNumber(317750010)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xEAedc32a51c510d35ebC11088fD5fF2b47aACF2E"),
                    // <https://snowscan.xyz/tx/0x101add261bd48e99eda133423c7b807912deefd15203d6c67d1b8018d0af354d>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59966208)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0xC49Ca921c4CD1117162eAEEc0ee969649997950c"),
                    // <https://basescan.org/tx/0x2bb5129b8c20751ab703c852f081e08c6623440e866ede8e1e1514694dad5e44>
                    deployment_information: Some(DeploymentInformation::BlockNumber(27852880)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x161f4014C27773840ccb4EC1957113e6DD028846"),
                    // <https://gnosisscan.io/tx/0x04965cda30a501e074b983c40c5ff83d70401597da929e937e39d60022f4f0d9>
                    deployment_information: Some(DeploymentInformation::BlockNumber(39136290)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xe42C2E153BB0A8899b59C73F5Ff941f9742F1197"),
                    // <https://etherscan.io/tx/0x31c205dc31a18ebac64ebea30bd5bf0189241a49154f17eafd68e1854b9cfa17>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22089887)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x268E2EE1413D768b6e2dc3F5a4ddc9Ae03d9AF42"),
                    // <https://optimistic.etherscan.io/tx/0x5b7223baa7212e5aaf49470e6a761133d9392d67c5e9d5d5c7ebc9c4719da601>
                    deployment_information: Some(DeploymentInformation::BlockNumber(133969860)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xc274A11E09a3c92Ac64eAff5bEC4ee8f5dfEe207"),
                    // <https://sepolia.etherscan.io/tx/0x48d803b01baf630543481ca6eefca5dc269d8670cf44afd08dcba3792a48710f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7944011)),
                },
            )
    });
    generate_contract_with_config("BalancerV3StableSurgePoolFactory", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x86e67E115f96DF37239E0479441303De0de7bc2b"),
                    // <https://arbiscan.io/tx/0x43eb1a286d4a06c767d780c3e7437f8f5cec1552b20d5fb717bb24f09c693924>
                    deployment_information: Some(DeploymentInformation::BlockNumber(303403113)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x4fb47126Fa83A8734991E41B942Ac29A3266C968"),
                    // <https://basescan.org/tx/0x38e5a884249f6afea6113cae9167a20f63ac1f6409edbf9da9d206ba4878f50a>
                    deployment_information: Some(DeploymentInformation::BlockNumber(26049433)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x268E2EE1413D768b6e2dc3F5a4ddc9Ae03d9AF42"),
                    // <https://gnosisscan.io/tx/0x05cbac83d6d1d75b5205a9ab6497acbbc48c33516f444ff0a70fb52e8185a11f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(38432088)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xD53F5d8d926fb2a0f7Be614B16e649B8aC102D83"),
                    // <https://etherscan.io/tx/0xea86300610bd6a6782395053c4f9cd5e428f4219a6416bc5b7bf6ea2c3998567>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21791079)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xD516c344413B4282dF1E4082EAE6B1081F3b1932"),
                    // <https://sepolia.etherscan.io/tx/0x813ed66325fdac564b4a4eeb9bb99058c0d82096325803cbe5319a473c0e00f0>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7655004)),
                },
            )
    });
    generate_contract_with_config("BalancerV3StableSurgePoolFactoryV2", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x201efd508c8DfE9DE1a13c2452863A78CB2a86Cc"),
                    // <https://arbiscan.io/tx/0xf0c872096b38df7396bdd796c7c44a8e073d10058a730d2393fecbceab7ae3e5>
                    deployment_information: Some(DeploymentInformation::BlockNumber(322937794)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x18CC3C68A5e64b40c846Aa6E45312cbcBb94f71b"),
                    // <https://snowscan.xyz/tx/0xa0d0795a93be94c92c7b5b7ab117a328e9183ee6387b5e5b7ddc5e7ded72abd0>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59966276)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x8e3fEaAB11b7B351e3EA1E01247Ab6ccc847dD52"),
                    // <https://basescan.org/tx/0x49603904b270ff5ce8efdc395a8c004683dcf64b1f75ae5b82461b40cd627041>
                    deployment_information: Some(DeploymentInformation::BlockNumber(28502516)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x45fB5aF0a1aD80Ea16C803146eb81844D9972373"),
                    // <https://gnosisscan.io/tx/0x317fd60d689b5146b9d9c93ef11fbe4a2caec8af69d8c05ed620033a27cf1a7f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(39390487)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x355bD33F0033066BB3DE396a6d069be57353AD95"),
                    // <https://etherscan.io/tx/0x7bd8f7b3744accd6595a5f6048f3165e4d60dd6ea951e5dd0c882bf193fd70c8>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22197594)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x3BEb058DE1A25dd24223fd9e1796df8589429AcE"),
                    // <https://optimistic.etherscan.io/tx/0x896531d84d833de10a86562a20a7cec4c40cb63fec2ea5691d75a7b3ae16ff10>
                    deployment_information: Some(DeploymentInformation::BlockNumber(134097700)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x2f1d6F4C40047dC122cA7e46B0D1eC27739BFc66"),
                    // <https://sepolia.etherscan.io/tx/0xb342f8518d64d9bb3f2436b369aa0dda8f3aadb46aa1c3228fa321519431a199>
                    deployment_information: Some(DeploymentInformation::BlockNumber(8050826)),
                },
            )
    });
    generate_contract_with_config("BalancerV3GyroECLPPoolFactory", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x88ED12A90142fDBFe2a28f7d5b48927254C7e760"),
                    // <https://arbiscan.io/tx/0x4d698081792d9437c064c3ce0509ca126f149027a3174e7aa6ebbd351f7bcd80>
                    deployment_information: Some(DeploymentInformation::BlockNumber(315658096)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x268E2EE1413D768b6e2dc3F5a4ddc9Ae03d9AF42"),
                    // <https://snowscan.xyz/tx/0x147f2acd80d5417dfe3004ab9f90e5c9ad6f4067e1c6993231d050c6efb0ee46>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59965989)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x5F6848976C2914403B425F18B589A65772F082E3"),
                    // <https://basescan.org/tx/0xe99692e0c80903e7b875cbb76a77febf86c10e054d3d98f1f886366101c33a22>
                    deployment_information: Some(DeploymentInformation::BlockNumber(27590349)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xEa924b45a3fcDAAdf4E5cFB1665823B8F8F2039B"),
                    // <https://gnosisscan.io/tx/0xfb731a5912f589b4123d32d6fa9a8817012760d8056e336dd4ecdc719f9e1892>
                    deployment_information: Some(DeploymentInformation::BlockNumber(39033094)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xE9B0a3bc48178D7FE2F5453C8bc1415d73F966d0"),
                    // <https://etherscan.io/tx/0x795e515da7dfd9b5f6c62fe95efe9c87063f68592805021154ff5ae870b57a09>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22046343)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x22625eEDd92c81a219A83e1dc48f88d54786B017"),
                    // <https://optimistic.etherscan.io/tx/0xeac4f4560a14aadb9ad0bece9884f3e527aa92d3fc35f67e380c2f20103ce696>
                    deployment_information: Some(DeploymentInformation::BlockNumber(133969692)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x589cA6855C348d831b394676c25B125BcdC7F8ce"),
                    // <https://sepolia.etherscan.io/tx/0xb9431fb3bec8a3a2320f63b1da9d96e62bd152b8fff4634cd92e0e3530f32783>
                    deployment_information: Some(DeploymentInformation::BlockNumber(7901684)),
                },
            )
    });
    generate_contract_with_config("BalancerV3Gyro2CLPPoolFactory", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x65A22Ec32c37835Ad5E77Eb6f7452Ac59E113a9F"),
                    // <https://arbiscan.io/tx/0xe7e1d42afe1fe3412db2675fcd95a1ff11686299bb03d1dda49cf1c8ed86b28b>
                    deployment_information: Some(DeploymentInformation::BlockNumber(322520182)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0xe2fa4e1d17725e72dcdAfe943Ecf45dF4B9E285b"),
                    // <https://snowscan.xyz/tx/0xb3fd1f08bb200e3dd9b61b4eca3f163b787a8f8bf317e4e1b3d70e27eb404a6f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59965891)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0xf5CDdF6feD9C589f1Be04899F48f9738531daD59"),
                    // <https://basescan.org/tx/0x54fcfff9e79b2b25acad56d29daa6f89111c4a43dfd9090ca6073f91df6b0d17>
                    deployment_information: Some(DeploymentInformation::BlockNumber(28450062)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x7fA49Df302a98223d98D115fc4FCD275576f6faA"),
                    // <https://gnosisscan.io/tx/0x6a9a7757c7808aef632b81e43c6847e987aa113623c88da5cbfea95e540e04fc>
                    deployment_information: Some(DeploymentInformation::BlockNumber(39369934)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xb96524227c4B5Ab908FC3d42005FE3B07abA40E9"),
                    // <https://etherscan.io/tx/0xaac5fd1c006e1f8c2e95d70923d6014d48f820eea19ac78248614db9bb2adbe3>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22188963)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x4b979eD48F982Ba0baA946cB69c1083eB799729c"),
                    // <https://optimistic.etherscan.io/tx/0x98dcb158b97fc79b0b447ffc86a5cb3e7f6a536e844818a5f58fd6f4fa991252>
                    deployment_information: Some(DeploymentInformation::BlockNumber(134045195)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x38ce8e04EBC04A39BED4b097e8C9bb8Ca74e33d8"),
                    // <https://sepolia.etherscan.io/tx/0xf84a5a835c02b5d6746dacf721b31dadab2631d98c600167976523ae86ae2d0a>
                    deployment_information: Some(DeploymentInformation::BlockNumber(8042511)),
                },
            )
    });
    generate_contract_with_config("BalancerV3ReClammPoolFactoryV2", |builder| {
        builder
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x355bD33F0033066BB3DE396a6d069be57353AD95"),
                    // <https://arbiscan.io/tx/0xb544a2bdea93f632fd739df575cc67bbb6d55e969b585fc93ba49b6a22bb5912>
                    deployment_information: Some(DeploymentInformation::BlockNumber(353502388)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x309abcAeFa19CA6d34f0D8ff4a4103317c138657"),
                    // <https://snowscan.xyz/tx/0x5d85462e695ff43bc6a3624c5a47ed7cd4e1057373c6ea7052e3a1c7cd16fc21>
                    deployment_information: Some(DeploymentInformation::BlockNumber(64832650)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x201efd508c8DfE9DE1a13c2452863A78CB2a86Cc"),
                    // <https://basescan.org/tx/0x1c3574deb31beba51d3b1cb5e45fede50cdf497793b54926f59ef883a2877f68>
                    deployment_information: Some(DeploymentInformation::BlockNumber(32339174)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xc86eF81E57492BE65BFCa9b0Ed53dCBAfDBe6100"),
                    // <https://gnosisscan.io/tx/0x61a5e5571a5e20ca3819abb7952f26194496cb1b87fcc9c6d36e4a03c663d704>
                    deployment_information: Some(DeploymentInformation::BlockNumber(40884126)),
                },
            )
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xDaa273AeEc06e9CCb7428a77E2abb1E4659B16D2"),
                    // <https://etherscan.io/tx/0x0e1c9630dd44a7e1d5c958b9e5d9c9e0b45888e54b9b3d24424675e849dc95e7>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22832233)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x891EC9B34829276a9a8ef2F8A9cEAF2486017e0d"),
                    // <https://optimistic.etherscan.io/tx/0xffb48a81db20156058aa6f81bbd8d53411887fd2e3a02f9fa24f4a3c748982cc>
                    deployment_information: Some(DeploymentInformation::BlockNumber(137934460)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xf58A574530Ea5cEB727095e6039170c1e8068fcA"),
                    // <https://sepolia.etherscan.io/tx/0xea5bbc9c461d578510096fcbc6ab0b3c78f1ff5c2343c34cb0848d9397a26e4e>
                    deployment_information: Some(DeploymentInformation::BlockNumber(8676768)),
                },
            )
    });
    generate_contract_with_config("BalancerV3QuantAMMWeightedPoolFactory", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0xD5c43063563f9448cE822789651662cA7DcD5773"),
                    // <https://etherscan.io/tx/0xf0836415bec5a29d4b338ef1c7f09cb070ec5db2e92b3c36903162844508aafc>
                    deployment_information: Some(DeploymentInformation::BlockNumber(22334706)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xe9B996395f9B6555426045d6A4d1087244d9490e"),
                    // <https://sepolia.etherscan.io/tx/0xd7702c5f889c1e20f035f253f725b6c34d6542e511b3b647c61fcf9ff2ee4bc4>
                    deployment_information: Some(DeploymentInformation::BlockNumber(8180675)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x62B9eC6A5BBEBe4F5C5f46C8A8880df857004295"),
                    // <https://arbiscan.io/tx/0x78424ecdd4fb61f320e4dced0cdf567843cc62cbde7bf56ee95f218a8bd0db3a>
                    deployment_information: Some(DeploymentInformation::BlockNumber(331549791)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x62B9eC6A5BBEBe4F5C5f46C8A8880df857004295"),
                    // <https://basescan.org/tx/0xda49be8739db416caa67fce44379fd760d2a162346c229a146e0ba121b06b078>
                    deployment_information: Some(DeploymentInformation::BlockNumber(29577953)),
                },
            )
    });
    generate_contract("BalancerV3WeightedPool");
    generate_contract("BalancerV3StablePool");
    generate_contract("BalancerV3StableSurgePool");
    generate_contract("BalancerV3StableSurgeHook");
    generate_contract("BalancerV3GyroECLPPool");
    generate_contract("BalancerV3Gyro2CLPPool");
    generate_contract("BalancerV3ReClammPool");
    generate_contract("BalancerV3QuantAMMWeightedPool");
    generate_contract("IRateProvider");

    generate_contract_with_config("BaoswapRouter", |builder| {
        builder.add_network_str(GNOSIS, "0x6093AeBAC87d62b1A5a4cEec91204e35020E38bE")
    });
    generate_contract("ERC20");
    generate_contract_with_config("FlashLoanRouter", |builder| {
        let mut builder = builder;
        for network in [
            MAINNET,
            GNOSIS,
            SEPOLIA,
            ARBITRUM_ONE,
            BASE,
            POLYGON,
            AVALANCHE,
        ] {
            builder = builder.add_network(
                network,
                Network {
                    address: addr("0x9da8b48441583a2b93e2ef8213aad0ec0b392c69"),
                    deployment_information: None,
                },
            );
        }
        builder
    });
    generate_contract_with_config("GPv2AllowListAuthentication", |builder| {
        builder
            .contract_mod_override("gpv2_allow_list_authentication")
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://etherscan.io/tx/0xb84bf720364f94c749f1ec1cdf0d4c44c70411b716459aaccfd24fc677013375>
                    deployment_information: Some(DeploymentInformation::BlockNumber(12593263)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://gnosisscan.io/tx/0x1a2d87a05a94bc6680a4faee31bbafbd74e9ddb63dd3941c717b5c609c08b957>
                    deployment_information: Some(DeploymentInformation::BlockNumber(16465099)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://sepolia.etherscan.io/tx/0x73c54c75b3f382304f3adf33e3876c8999fb10df786d4a902733369251033cd1>
                    deployment_information: Some(DeploymentInformation::BlockNumber(4717469)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://arbiscan.io/tx/0xe994adff141a2e72bd9dab3eb7b3480637013bdfb1aa42c62d9d6c90de091237>
                    deployment_information: Some(DeploymentInformation::BlockNumber(204702129)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://basescan.org/tx/0x5497004d2a37c9eafd0bd1e5861a67d3a209c5b845724166e3dbca9527ee05ec>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21407137)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://snowscan.xyz/tx/0xa58fc76846917779d7bcbb7d34f4a2a44aab2b702ef983594e34e6972a0c626b>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59891351)),
                },
            )
            .add_network(
                BNB,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://bscscan.com/tx/0x8da639c62eb4a810573c178ed245184944d66c834122e3f88994ebf679b50e34>
                    deployment_information: Some(DeploymentInformation::BlockNumber(48173639)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://optimistic.etherscan.io/tx/0x5b6403b485e369ce524d04234807df782e6639e55a7c1d859f0a67925d9a5f49>
                    deployment_information: Some(DeploymentInformation::BlockNumber(134254466)),
                },
            )
            .add_network(
                POLYGON,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://polygonscan.com/tx/0x686e4bbcfd6ebae91f0fcc667407c831953629877ec622457916729de3d461c3>
                    deployment_information: Some(DeploymentInformation::BlockNumber(45854728)),
                },
            )
            .add_network(
                LENS,
                Network {
                    address: addr("0x2c4c28DDBdAc9C5E7055b4C863b72eA0149D8aFE"),
                    // <https://explorer.lens.xyz/tx/0x0730c21885153dcc9a25ab7abdc38309ec7c7a8db15b763fbbaf574d1e7ec498>
                    deployment_information: Some(DeploymentInformation::BlockNumber(2612937)),
                },
            )
    });
    generate_contract_with_config("GPv2Settlement", |builder| {
        builder
            .contract_mod_override("gpv2_settlement")
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://etherscan.io/tx/0xf49f90aa5a268c40001d1227b76bb4dd8247f18361fcad9fffd4a7a44f1320d3>
                    deployment_information: Some(DeploymentInformation::BlockNumber(12593265)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://blockscout.com/xdai/mainnet/tx/0x9ddc538f89cd8433f4a19bc4de0de27e7c68a1d04a14b327185e4bba9af87133>
                    deployment_information: Some(DeploymentInformation::BlockNumber(16465100)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://sepolia.etherscan.io/tx/0x6bba22a00ffcff6bca79aced546e18d2a5a4f4e484a4e4dbafab13daf42f718d>
                    deployment_information: Some(DeploymentInformation::BlockNumber(4717488)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    deployment_information: Some(DeploymentInformation::BlockNumber(204704802)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://basescan.org/tx/0x00a3c4e2dc4241465208beeba27e90a9ce3159ad4f41581c4c3a1ef02d6e37cb>
                    deployment_information: Some(DeploymentInformation::BlockNumber(21407238)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://snowscan.xyz/tx/0x374b84f0ea6bc554abc3ffdc3fbce4374fefc76f2bd25e324ce95a62cafcc142>
                    deployment_information: Some(DeploymentInformation::BlockNumber(59891356)),
                },
            )
            .add_network(
                BNB,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://bscscan.com/tx/0x9e0c16a655ceadcb95ba2de3bf59d2b3a3d10cce7bdf52aa5520164b58ffd969>
                    deployment_information: Some(DeploymentInformation::BlockNumber(48173641)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://optimistic.etherscan.io/tx/0xd1bbd68ee6b0eecf6f883e148284fc4fb4c960299b75004dfddd5135246cd5eb>
                    deployment_information: Some(DeploymentInformation::BlockNumber(134254624)),
                },
            )
            .add_network(
                POLYGON,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://polygonscan.com/tx/0x0e24d3a2a8530eaad5ae62e54e64d57665a77ce3970227d20c1b77da315cbbf6>
                    deployment_information: Some(DeploymentInformation::BlockNumber(45859743)),
                },
            )
            .add_network(
                LENS,
                Network {
                    address: addr("0x9008D19f58AAbD9eD0D60971565AA8510560ab41"),
                    // <https://explorer.lens.xyz/tx/0x01584b767dda7b115394b93dbcfecadfe589862ae3f7957846a2db82f2f5c703>
                    deployment_information: Some(DeploymentInformation::BlockNumber(2621745)),
                },
            )
    });
    generate_contract_with_config("HoneyswapRouter", |builder| {
        builder.add_network_str(GNOSIS, "0x1C232F01118CB8B424793ae03F870aa7D0ac7f77")
    });
    // EIP-1271 contract - SignatureValidator
    generate_contract("ERC1271SignatureValidator");

    generate_contract_with_config("UniswapV3SwapRouterV2", |builder| {
        // <https://github.com/Uniswap/v3-periphery/blob/697c2474757ea89fec12a4e6db16a574fe259610/deploys.md>
        builder
            .add_network_str(MAINNET, "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")
            .add_network_str(SEPOLIA, "0xE592427A0AEce92De3Edee1F18E0157C05861564")
            .add_network_str(ARBITRUM_ONE, "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")
            // For Base, Avalanche and BNB it is only available SwapRouter02
            // <https://docs.uniswap.org/contracts/v3/reference/deployments/base-deployments>
            .add_network_str(POLYGON, "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")
            .add_network_str(OPTIMISM, "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")
            .add_network_str(BASE, "0x2626664c2603336E57B271c5C0b26F421741e481")
            .add_network_str(AVALANCHE, "0xbb00FF08d01D300023C629E8fFfFcb65A5a578cE")
            .add_network_str(BNB, "0xB971eF87ede563556b2ED4b1C0b0019111Dd85d2")
            .add_network_str(LENS, "0x6ddD32cd941041D8b61df213B9f515A7D288Dc13")
        // Not available on Gnosis Chain
    });
    generate_contract_with_config("UniswapV3QuoterV2", |builder| {
        // <https://docs.uniswap.org/contracts/v3/reference/deployments/>
        builder
            .add_network_str(MAINNET, "0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
            .add_network_str(ARBITRUM_ONE, "0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
            .add_network_str(BASE, "0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a")
            .add_network_str(AVALANCHE, "0xbe0F5544EC67e9B3b2D979aaA43f18Fd87E6257F")
            .add_network_str(BNB, "0x78D78E420Da98ad378D7799bE8f4AF69033EB077")
            .add_network_str(OPTIMISM, "0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
            .add_network_str(POLYGON, "0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
            .add_network_str(LENS, "0x1eEA2B790Dc527c5a4cd3d4f3ae8A2DDB65B2af1")
        // Not listed on Gnosis and Sepolia chains
    });
    generate_contract("IERC4626");
    generate_contract_with_config("WETH9", |builder| {
        // Note: the WETH address must be consistent with the one used by the ETH-flow
        // contract
        builder
            .add_network_str(MAINNET, "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
            .add_network_str(GNOSIS, "0xe91D153E0b41518A2Ce8Dd3D7944Fa863463a97d")
            .add_network_str(SEPOLIA, "0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14")
            .add_network_str(ARBITRUM_ONE, "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1")
            .add_network_str(BASE, "0x4200000000000000000000000000000000000006")
            .add_network_str(AVALANCHE, "0xB31f66AA3C1e785363F0875A1B74E27b85FD66c7")
            .add_network_str(BNB, "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c")
            .add_network_str(OPTIMISM, "0x4200000000000000000000000000000000000006")
            .add_network_str(POLYGON, "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270")
            .add_network_str(LENS, "0x6bDc36E20D267Ff0dd6097799f82e78907105e2F")
    });
    generate_contract_with_config("IUniswapV3Factory", |builder| {
        // <https://github.com/Uniswap/v3-periphery/blob/697c2474757ea89fec12a4e6db16a574fe259610/deploys.md>
        builder
            .add_network_str(MAINNET, "0x1F98431c8aD98523631AE4a59f267346ea31F984")
            .add_network_str(SEPOLIA, "0x1F98431c8aD98523631AE4a59f267346ea31F984")
            .add_network_str(ARBITRUM_ONE, "0x1F98431c8aD98523631AE4a59f267346ea31F984")
            .add_network_str(BASE, "0x33128a8fC17869897dcE68Ed026d694621f6FDfD")
            .add_network_str(AVALANCHE, "0x740b1c1de25031C31FF4fC9A62f554A55cdC1baD")
            .add_network_str(BNB, "0xdB1d10011AD0Ff90774D0C6Bb92e5C5c8b4461F7")
            .add_network_str(OPTIMISM, "0x1F98431c8aD98523631AE4a59f267346ea31F984")
            .add_network_str(POLYGON, "0x1F98431c8aD98523631AE4a59f267346ea31F984")
            // not official
            .add_network_str(LENS, "0xc3A5b857Ba82a2586A45a8B59ECc3AA50Bc3D0e3")
        // Not available on Gnosis Chain
    });
    generate_contract_with_config("CowProtocolToken", |builder| {
        builder
            .add_network_str(MAINNET, "0xDEf1CA1fb7FBcDC777520aa7f396b4E015F497aB")
            .add_network_str(GNOSIS, "0x177127622c4A00F3d409B75571e12cB3c8973d3c")
            .add_network_str(SEPOLIA, "0x0625aFB445C3B6B7B929342a04A22599fd5dBB59")
            .add_network_str(ARBITRUM_ONE, "0xcb8b5CD20BdCaea9a010aC1F8d835824F5C87A04")
            .add_network_str(BASE, "0xc694a91e6b071bF030A18BD3053A7fE09B6DaE69")
        // Not available on Lens
    });
    generate_contract("CowAmm");
    generate_contract_with_config("CowAmmConstantProductFactory", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x40664207e3375FB4b733d4743CE9b159331fd034"),
                    // <https://etherscan.io/tx/0xf37fc438ddacb00c28305bd7dea3b79091cd5be3405a2b445717d9faf946fa50>
                    deployment_information: Some(DeploymentInformation::BlockNumber(19861952)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xdb1cba3a87f2db53b6e1e6af48e28ed877592ec0"),
                    // <https://gnosisscan.io/tx/0x4121efab4ad58ae7ad73b50448cccae0de92905e181648e5e08de3d6d9c66083>
                    deployment_information: Some(DeploymentInformation::BlockNumber(33874317)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0xb808e8183e3a72d196457d127c7fd4befa0d7fd3"),
                    // <https://sepolia.etherscan.io/tx/0x5e6af00c670eb421b96e78fd2e3b9df573b19e6e0ea77d8003e47cdde384b048>
                    deployment_information: Some(DeploymentInformation::BlockNumber(5874562)),
                },
            )
    });
    generate_contract_with_config("CowAmmLegacyHelper", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x3705ceee5eaa561e3157cf92641ce28c45a3999c"),
                    // <https://etherscan.io/tx/0x07f0ce50fb9cd30e69799a63ae9100869a3c653d62ea3ba49d2e5e1282f42b63>
                    deployment_information: Some(DeploymentInformation::BlockNumber(20332745)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0xd9ec06b001957498ab1bc716145515d1d0e30ffb"),
                    // <https://gnosisscan.io/tx/0x09e56c7173ab1e1c5d02bc2832799422ebca6d9a40e5bae77f6ca908696bfebf>
                    deployment_information: Some(DeploymentInformation::BlockNumber(35026999)),
                },
            )
    });
    generate_contract("CowAmmUniswapV2PriceOracle");

    // Support contracts used for various order simulations.
    generate_contract_with_config("Balances", |builder| {
        builder
            .add_network_str(MAINNET, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(ARBITRUM_ONE, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(BASE, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(AVALANCHE, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(BNB, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(OPTIMISM, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(POLYGON, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(LENS, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(GNOSIS, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
            .add_network_str(SEPOLIA, "0x3e8C6De9510e7ECad902D005DE3Ab52f35cF4f1b")
    });
    generate_contract_with_config("Signatures", |builder| {
        builder
            .add_network_str(MAINNET, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(ARBITRUM_ONE, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(BASE, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(AVALANCHE, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(BNB, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(OPTIMISM, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(POLYGON, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(LENS, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(GNOSIS, "0x8262d639c38470F38d2eff15926F7071c28057Af")
            .add_network_str(SEPOLIA, "0x8262d639c38470F38d2eff15926F7071c28057Af")
    });

    // Contract for batching multiple `eth_call`s into a single one.
    generate_contract("Multicall");

    // Test Contract for incrementing arbitrary counters.
    generate_contract("Counter");

    // Test Contract for using up a specified amount of gas.
    generate_contract("GasHog");

    // Contract for Uniswap's Permit2 contract.
    generate_contract_with_config("Permit2", |builder| {
        builder
            .add_network(
                MAINNET,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://etherscan.io/tx/0xf2f1fe96c16ee674bb7fcee166be52465a418927d124f5f1d231b36eae65d377>
                    deployment_information: Some(DeploymentInformation::BlockNumber(15986406)),
                },
            )
            .add_network(
                GNOSIS,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://gnosisscan.io/tx/0x3ba511410edc92cafe94bd100e25adb37981499d17947a3d64c8523fbfd31864>
                    deployment_information: Some(DeploymentInformation::BlockNumber(27338672)),
                },
            )
            .add_network(
                SEPOLIA,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://sepolia.etherscan.io/tx/0x363df5deeead44d8fd38425f3986e3e81946a6c59d8b68fe33926cc700713173>
                    deployment_information: Some(DeploymentInformation::BlockNumber(2356287)),
                },
            )
            .add_network(
                ARBITRUM_ONE,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://arbiscan.io/tx/0xe244dafca8211ed6fb123efaa5075b7d5813749718412ca435c872afd0e2ea82>
                    deployment_information: Some(DeploymentInformation::BlockNumber(38692735)),
                },
            )
            .add_network(
                BASE,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://basescan.org/tx/0x26fbdea9a47ba8e21676bc6b6a72a19dded1a0c270e96d5236886ca9c5000d3f>
                    deployment_information: Some(DeploymentInformation::BlockNumber(1425180)),
                },
            )
            .add_network(
                AVALANCHE,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://snowscan.xyz/tx/0x38fd76c2165d920c7e006defd67eeeb0069bf93e41741eec3bbb83d196610a56>
                    deployment_information: Some(DeploymentInformation::BlockNumber(28844415)),
                },
            )
            .add_network(
                BNB,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://bscscan.com/tx/0xb038ec7b72db4207e0c0d5433e1cabc41b4e4f9b9cac577173b3188fc508a6c3>
                    deployment_information: Some(DeploymentInformation::BlockNumber(25343783)),
                },
            )
            .add_network(
                OPTIMISM,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://optimistic.etherscan.io/tx/0xf0a51e0d0579ef8cc7965f5797bd7665ddac14d4d2141423676b8862f7668352>
                    deployment_information: Some(DeploymentInformation::BlockNumber(38854427)),
                },
            )
            .add_network(
                POLYGON,
                Network {
                    address: addr("0x000000000022D473030F116dDEE9F6B43aC78BA3"),
                    // <https://polygonscan.com/tx/0xe2a4d996de0d6a23108f701b37acba6c47ee34448bb51fec5c23f542a6f3ccc8>
                    deployment_information: Some(DeploymentInformation::BlockNumber(35701901)),
                },
            )
        // Not available on Lens
    });
}

fn generate_contract(name: &str) {
    generate_contract_with_config(name, |builder| builder)
}

fn generate_contract_with_config(
    name: &str,
    config: impl FnOnce(ContractBuilder) -> ContractBuilder,
) {
    let path = paths::contract_artifacts_dir()
        .join(name)
        .with_extension("json");
    let contract = TruffleLoader::new()
        .name(name)
        .load_contract_from_file(&path)
        .unwrap();
    let dest = env::var("OUT_DIR").unwrap();

    println!("cargo:rerun-if-changed={}", path.display());

    config(ContractBuilder::new().visibility_modifier("pub"))
        .generate(&contract)
        .unwrap()
        .write_to_file(Path::new(&dest).join(format!("{name}.rs")))
        .unwrap();
}

fn addr(s: &str) -> Address {
    s.parse().unwrap()
}
