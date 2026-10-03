//! Ergo Explorer API client (public and p2p instances share the API).
use std::collections::HashSet;

use serde::Deserialize;

use super::{get_json, Result, SourceError};
use crate::model::{find_miner_reward, Block, BlockRef, BoxData, Input, Token, Tx};

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(default)]
    total: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetJson {
    token_id: String,
    amount: u64,
}

#[derive(Deserialize)]
struct InputJson {
    id: String,
    #[serde(default)]
    value: Option<u64>,
    #[serde(default)]
    address: Option<String>,
}

#[derive(Deserialize)]
struct OutputJson {
    id: String,
    value: u64,
    address: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxJson {
    id: String,
    #[serde(default)]
    inputs: Vec<InputJson>,
    #[serde(default)]
    outputs: Vec<OutputJson>,
    #[serde(default)]
    creation_timestamp: Option<u64>,
    #[serde(default)]
    size: Option<u32>,
}

#[derive(Deserialize)]
struct BlockSummaryJson {
    id: String,
    height: u32,
    timestamp: u64,
}

#[derive(Deserialize)]
struct HeaderJson {
    id: String,
    height: u32,
    timestamp: u64,
    #[serde(default)]
    size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockBodyJson {
    header: HeaderJson,
    block_transactions: Vec<TxJson>,
}

#[derive(Deserialize)]
struct BlockJson {
    block: BlockBodyJson,
}

fn to_tx(t: TxJson) -> Tx {
    Tx {
        id: t.id,
        size: t.size.unwrap_or(0),
        inputs: t
            .inputs
            .into_iter()
            .map(|i| {
                let resolved = match (i.value, i.address) {
                    (Some(value), Some(address)) => Some(BoxData {
                        box_id: i.id.clone(),
                        value,
                        address,
                        tokens: vec![],
                    }),
                    _ => None,
                };
                Input {
                    box_id: i.id,
                    resolved,
                }
            })
            .collect(),
        outputs: t
            .outputs
            .into_iter()
            .map(|o| BoxData {
                box_id: o.id,
                value: o.value,
                address: o.address,
                tokens: o
                    .assets
                    .into_iter()
                    .map(|a| Token {
                        token_id: a.token_id,
                        amount: a.amount,
                    })
                    .collect(),
            })
            .collect(),
        creation_ts_ms: t.creation_timestamp,
    }
}

/// Page sizes tried, in order, when the explorer returns nothing for one large page.
const SMALL_PAGE_SIZES: [u32; 3] = [10, 5, 2];
/// At most this many txs fetched through small pages per poll.
const SMALL_MAX_TXS: u32 = 500;

#[derive(Clone, Debug)]
pub struct MempoolSnapshot {
    pub txs: Vec<Tx>,
    /// The explorer's own count, which can exceed what it actually served.
    pub total: u64,
}

impl MempoolSnapshot {
    pub fn is_partial(&self) -> bool {
        (self.txs.len() as u64) < self.total
    }
}

pub struct ExplorerClient {
    http: reqwest::Client,
    base: String,
}

impl ExplorerClient {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// The explorer's mempool. Public explorers sometimes return an empty list for large
    /// pages while still reporting a non-zero total; then this falls back to small pages.
    pub async fn mempool(&self) -> Result<MempoolSnapshot> {
        let url = format!(
            "{}/transactions/unconfirmed?limit=10000&offset=0",
            self.base
        );
        let page: Page<TxJson> = get_json(&self.http, &url).await?;
        let mut total = page.total.unwrap_or(page.items.len() as u64);
        if !page.items.is_empty() || total == 0 {
            let txs: Vec<Tx> = page.items.into_iter().map(to_tx).collect();
            return Ok(MempoolSnapshot {
                total: total.max(txs.len() as u64),
                txs,
            });
        }
        let mut seen = HashSet::new();
        let mut txs = Vec::new();
        // The largest page an explorer backend will answer varies (seen: 10, then only 5),
        // so use the first size whose first page returns data.
        for size in SMALL_PAGE_SIZES {
            for i in 0..SMALL_MAX_TXS / size {
                let url = format!(
                    "{}/transactions/unconfirmed?limit={size}&offset={}",
                    self.base,
                    i * size
                );
                let page: Page<TxJson> = get_json(&self.http, &url).await?;
                total = total.max(page.total.unwrap_or(0));
                let n = page.items.len();
                for t in page.items {
                    if seen.insert(t.id.clone()) {
                        txs.push(to_tx(t));
                    }
                }
                if n < size as usize || txs.len() as u64 >= total {
                    break;
                }
            }
            if !txs.is_empty() {
                break;
            }
        }
        if txs.is_empty() {
            return Err(SourceError::Parse(format!(
                "explorer reported {total} unconfirmed txs but returned none"
            )));
        }
        Ok(MempoolSnapshot {
            total: total.max(txs.len() as u64),
            txs,
        })
    }

    pub async fn latest_blocks(&self, n: u32) -> Result<Vec<BlockRef>> {
        let url = format!("{}/api/v1/blocks?limit={n}", self.base);
        let page: Page<BlockSummaryJson> = get_json(&self.http, &url).await?;
        Ok(page
            .items
            .into_iter()
            .map(|b| BlockRef {
                id: b.id,
                height: b.height,
                timestamp_ms: b.timestamp,
            })
            .collect())
    }

    pub async fn block(&self, id: &str) -> Result<Block> {
        let b: BlockJson =
            get_json(&self.http, &format!("{}/api/v1/blocks/{id}", self.base)).await?;
        let txs: Vec<Tx> = b.block.block_transactions.into_iter().map(to_tx).collect();
        let reward = txs.first().and_then(|t| find_miner_reward(&t.outputs));
        let h = b.block.header;
        Ok(Block {
            id: h.id,
            height: h.height,
            timestamp_ms: h.timestamp,
            size: h.size,
            miner_address: reward.map(|r| r.address.clone()),
            miner_reward: reward.map(|r| r.value).unwrap_or(0),
            tx_ids: txs.iter().map(|t| t.id.clone()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNCONFIRMED: &str = include_str!("../../tests/fixtures/explorer/unconfirmed.json");
    const BLOCKS: &str = include_str!("../../tests/fixtures/explorer/blocks.json");
    const BLOCK: &str = include_str!("../../tests/fixtures/explorer/block.json");

    async fn mock(server: &MockServer, p: &str, status: u16, body: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn parses_mempool() {
        let s = MockServer::start().await;
        mock(&s, "/transactions/unconfirmed", 200, UNCONFIRMED).await;
        let txs = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap()
            .txs;
        assert_eq!(txs.len(), 1);
        let t = &txs[0];
        assert_eq!(t.id, "e1");
        assert_eq!(t.size, 412);
        assert_eq!(t.creation_ts_ms, Some(1790978000000));
        let input = t.inputs[0].resolved.as_ref().unwrap();
        assert_eq!(
            input.address,
            "9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1"
        );
        assert_eq!(input.value, 11889000000);
        assert_eq!(t.outputs[0].address, "4MQyMKvMbnCJG3aJ");
        assert_eq!(t.outputs[0].tokens[0].amount, 5);
        let m = crate::metrics::tx_metrics(t);
        assert_eq!((m.fee, m.value, m.approx), (1500000, 11887500000, false));
    }

    #[tokio::test]
    async fn parses_blocks_and_block_detail() {
        let s = MockServer::start().await;
        mock(&s, "/api/v1/blocks", 200, BLOCKS).await;
        mock(&s, "/api/v1/blocks/blk-2", 200, BLOCK).await;
        let c = ExplorerClient::new(http_client(), &s.uri());
        let refs = c.latest_blocks(2).await.unwrap();
        assert_eq!(
            refs[0],
            BlockRef {
                id: "blk-2".into(),
                height: 1886101,
                timestamp_ms: 1790978083213
            }
        );
        let b = c.block("blk-2").await.unwrap();
        assert_eq!(b.tx_ids, vec!["cb-1", "e1"]);
        assert_eq!(b.size, 187236);
        assert_eq!(b.miner_reward, 12000000000);
        assert!(b.miner_address.unwrap().starts_with("88dhgz"));
    }

    #[tokio::test]
    async fn http_error_is_reported() {
        let s = MockServer::start().await;
        mock(&s, "/transactions/unconfirmed", 503, "").await;
        let err = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap_err();
        assert!(matches!(err, crate::sources::SourceError::Status(503)));
    }

    #[tokio::test]
    async fn items_missing_despite_total_is_an_error() {
        let s = MockServer::start().await;
        mock(
            &s,
            "/transactions/unconfirmed",
            200,
            r#"{"items":[],"total":70}"#,
        )
        .await;
        let err = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap_err();
        assert!(
            matches!(err, crate::sources::SourceError::Parse(_)),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn genuinely_empty_mempool_is_ok() {
        let s = MockServer::start().await;
        mock(
            &s,
            "/transactions/unconfirmed",
            200,
            r#"{"items":[],"total":0}"#,
        )
        .await;
        let txs = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap()
            .txs;
        assert!(txs.is_empty());
    }

    #[tokio::test]
    async fn pages_through_small_requests_when_the_big_page_comes_back_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/transactions/unconfirmed"))
            .and(query_param("limit", "10000"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"items":[],"total":3}"#))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/transactions/unconfirmed"))
            .and(query_param("limit", "10"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_string(UNCONFIRMED))
            .mount(&s)
            .await;
        let snap = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap();
        assert_eq!(
            snap.txs.len(),
            1,
            "small page served the tx the big page hid"
        );
        assert_eq!(snap.total, 3, "keeps the explorer's claimed total");
        assert!(snap.is_partial());
    }

    #[tokio::test]
    async fn a_full_big_page_is_not_partial() {
        let s = MockServer::start().await;
        mock(&s, "/transactions/unconfirmed", 200, UNCONFIRMED).await;
        let snap = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap();
        assert_eq!((snap.txs.len(), snap.total), (1, 1));
        assert!(!snap.is_partial());
    }

    #[tokio::test]
    async fn shrinks_the_page_size_until_the_explorer_answers() {
        let s = MockServer::start().await;
        for limit in ["10000", "10"] {
            Mock::given(method("GET"))
                .and(path("/transactions/unconfirmed"))
                .and(query_param("limit", limit))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(r#"{"items":[],"total":3}"#),
                )
                .mount(&s)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/transactions/unconfirmed"))
            .and(query_param("limit", "5"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_string(UNCONFIRMED))
            .mount(&s)
            .await;
        let snap = ExplorerClient::new(http_client(), &s.uri())
            .mempool()
            .await
            .unwrap();
        assert_eq!((snap.txs.len(), snap.total), (1, 3));
    }
}
