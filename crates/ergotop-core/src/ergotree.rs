//! Ergo mainnet ErgoTree <-> address conversion (P2PK and P2S).
//!
//! Address bytes = [network|type] ++ content ++ checksum, where checksum is the
//! first 4 bytes of blake2b256([network|type] ++ content).
use blake2::{digest::consts::U32, Blake2b, Digest};

type Blake2b256 = Blake2b<U32>;

const MAINNET_P2PK: u8 = 0x01;
const MAINNET_P2S: u8 = 0x03;
const P2PK_TREE_PREFIX: [u8; 3] = [0x00, 0x08, 0xcd];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("invalid hex")]
    Hex,
    #[error("invalid base58")]
    Base58,
    #[error("address too short")]
    TooShort,
    #[error("checksum mismatch")]
    Checksum,
    #[error("unsupported address type byte {0:#04x}")]
    Unsupported(u8),
}

fn checksum(body: &[u8]) -> [u8; 4] {
    let digest = Blake2b256::digest(body);
    [digest[0], digest[1], digest[2], digest[3]]
}

pub fn tree_bytes_to_address(tree: &[u8]) -> String {
    let mut body = Vec::with_capacity(tree.len() + 5);
    if tree.len() == 36 && tree[..3] == P2PK_TREE_PREFIX {
        body.push(MAINNET_P2PK);
        body.extend_from_slice(&tree[3..]);
    } else {
        body.push(MAINNET_P2S);
        body.extend_from_slice(tree);
    }
    let cs = checksum(&body);
    body.extend_from_slice(&cs);
    bs58::encode(body).into_string()
}

pub fn tree_to_address(tree_hex: &str) -> Result<String, AddressError> {
    let tree = hex::decode(tree_hex).map_err(|_| AddressError::Hex)?;
    Ok(tree_bytes_to_address(&tree))
}

pub fn address_to_tree(address: &str) -> Result<String, AddressError> {
    let bytes = bs58::decode(address)
        .into_vec()
        .map_err(|_| AddressError::Base58)?;
    if bytes.len() < 5 {
        return Err(AddressError::TooShort);
    }
    let (body, cs) = bytes.split_at(bytes.len() - 4);
    if checksum(body)[..] != *cs {
        return Err(AddressError::Checksum);
    }
    match body[0] {
        MAINNET_P2PK if body.len() == 34 => Ok(format!("0008cd{}", hex::encode(&body[1..]))),
        MAINNET_P2S => Ok(hex::encode(&body[1..])),
        t => Err(AddressError::Unsupported(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P2PK_ADDR: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const P2PK_TREE: &str =
        "0008cd033a8238d69857709e47016aa51b06d0c95d67b8855c23e24c0d8fb26667424e76";
    const FEE_ADDR: &str = "2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe";
    const FEE_TREE: &str = "1005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304";
    const SHORT_ADDR: &str = "4MQyMKvMbnCJG3aJ";
    const SHORT_TREE: &str = "10010100d17300";
    const POOL_ADDR: &str =
        "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY";
    const POOL_TREE: &str = "100204a00b08cd0274e729bb6615cbda94d9d176a2f1525068f12b330e38bbbf387232797dfd891fea02d192a39a8cc7a70173007301";

    #[test]
    fn tree_to_address_matches_vectors() {
        assert_eq!(tree_to_address(P2PK_TREE).unwrap(), P2PK_ADDR);
        assert_eq!(tree_to_address(FEE_TREE).unwrap(), FEE_ADDR);
        assert_eq!(tree_to_address(SHORT_TREE).unwrap(), SHORT_ADDR);
        assert_eq!(tree_to_address(POOL_TREE).unwrap(), POOL_ADDR);
    }

    #[test]
    fn address_to_tree_matches_vectors() {
        assert_eq!(address_to_tree(P2PK_ADDR).unwrap(), P2PK_TREE);
        assert_eq!(address_to_tree(FEE_ADDR).unwrap(), FEE_TREE);
        assert_eq!(address_to_tree(SHORT_ADDR).unwrap(), SHORT_TREE);
        assert_eq!(address_to_tree(POOL_ADDR).unwrap(), POOL_TREE);
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut bad = P2PK_ADDR.to_string();
        bad.pop();
        bad.push('r');
        assert_eq!(address_to_tree(&bad), Err(AddressError::Checksum));
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(address_to_tree("0OIl"), Err(AddressError::Base58));
        assert_eq!(address_to_tree("1"), Err(AddressError::TooShort));
        assert_eq!(tree_to_address("zz"), Err(AddressError::Hex));
    }

    #[test]
    fn rejects_p2sh() {
        let mut body = vec![0x02u8];
        body.extend_from_slice(&[0u8; 24]);
        let cs = checksum(&body);
        body.extend_from_slice(&cs);
        let addr = bs58::encode(body).into_string();
        assert_eq!(address_to_tree(&addr), Err(AddressError::Unsupported(0x02)));
    }
}
