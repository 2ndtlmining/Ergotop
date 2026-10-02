//! Core data types. All ERG amounts are nanoERG.

pub const NANOERG_PER_ERG: u64 = 1_000_000_000;

/// Miner reward boxes are locked by a P2S contract whose address starts with this.
pub const MINER_REWARD_PREFIX: &str = "88dhgzEuTXa";

pub type TxId = String;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub String);

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Node,
    Explorer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Unknown,
    Up,
    Degraded(String),
    Down(String),
}

impl SourceStatus {
    pub fn usable(&self) -> bool {
        matches!(self, SourceStatus::Up | SourceStatus::Degraded(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub token_id: String,
    pub amount: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxData {
    pub box_id: String,
    pub value: u64,
    pub address: String,
    pub tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub box_id: String,
    pub resolved: Option<BoxData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    pub id: TxId,
    pub size: u32,
    pub inputs: Vec<Input>,
    pub outputs: Vec<BoxData>,
    pub creation_ts_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRef {
    pub id: String,
    pub height: u32,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub height: u32,
    pub timestamp_ms: u64,
    pub size: u32,
    pub tx_ids: Vec<TxId>,
    pub miner_address: Option<String>,
    pub miner_reward: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeInfo {
    pub full_height: u32,
    pub headers_height: u32,
    pub peers: u32,
    pub app_version: String,
    pub max_block_size: u32,
    pub indexed_height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenMeta {
    pub token_id: String,
    pub name: Option<String>,
    pub decimals: u32,
}

pub fn nano_to_erg(n: u64) -> f64 {
    n as f64 / NANOERG_PER_ERG as f64
}

/// The miner reward box among a block's first (emission) transaction outputs.
pub fn find_miner_reward(outputs: &[BoxData]) -> Option<&BoxData> {
    outputs
        .iter()
        .find(|o| o.address.starts_with(MINER_REWARD_PREFIX))
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub fn bx(address: &str, value: u64) -> BoxData {
        BoxData {
            box_id: format!("box-{address}-{value}"),
            value,
            address: address.to_string(),
            tokens: vec![],
        }
    }

    /// A tx whose inputs are all resolved to the given boxes.
    pub fn tx(id: &str, size: u32, inputs: Vec<BoxData>, outputs: Vec<BoxData>) -> Tx {
        Tx {
            id: id.to_string(),
            size,
            inputs: inputs
                .into_iter()
                .map(|b| Input {
                    box_id: b.box_id.clone(),
                    resolved: Some(b),
                })
                .collect(),
            outputs,
            creation_ts_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_nanoerg_to_erg() {
        assert_eq!(nano_to_erg(1_500_000_000), 1.5);
        assert_eq!(nano_to_erg(0), 0.0);
    }

    #[test]
    fn finds_miner_reward_output() {
        let outs = vec![
            test_util::bx("2Z4YBkDsDvQj8B", 1_170_924_000_000_000),
            test_util::bx(
                "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY",
                12_000_000_000,
            ),
        ];
        let reward = find_miner_reward(&outs).expect("reward output");
        assert_eq!(reward.value, 12_000_000_000);
        assert!(find_miner_reward(&outs[..1]).is_none());
    }

    #[test]
    fn usable_statuses() {
        assert!(SourceStatus::Up.usable());
        assert!(SourceStatus::Degraded("lag".into()).usable());
        assert!(!SourceStatus::Down("x".into()).usable());
        assert!(!SourceStatus::Unknown.usable());
    }
}
