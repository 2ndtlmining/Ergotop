//! Ergo node REST client.
use std::collections::HashMap;

use serde::Deserialize;

use super::{get_json, post_json, Result};
use crate::ergotree::tree_to_address;
use crate::model::{
    find_miner_reward, Block, BlockRef, BoxData, Input, NodeInfo, Token, TokenMeta, Tx, TxId,
};

const BOX_CHUNK: usize = 100;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoJson {
    #[serde(default)]
    full_height: Option<u32>,
    #[serde(default)]
    headers_height: Option<u32>,
    #[serde(default)]
    peers_count: Option<u32>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    parameters: Option<ParamsJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParamsJson {
    max_block_size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexedHeightJson {
    indexed_height: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputJson {
    box_id: String,
    value: u64,
    ergo_tree: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetJson {
    token_id: String,
    amount: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputJson {
    box_id: String,
}

#[derive(Deserialize)]
struct TxJson {
    id: String,
    inputs: Vec<InputJson>,
    outputs: Vec<OutputJson>,
    #[serde(default)]
    size: Option<u32>,
}

#[derive(Deserialize)]
struct HeaderJson {
    id: String,
    height: u32,
    timestamp: u64,
}

#[derive(Deserialize)]
struct BlockTxsJson {
    transactions: Vec<TxJson>,
    #[serde(default)]
    size: u32,
}

#[derive(Deserialize)]
struct TokenJson {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    decimals: Option<u32>,
}

fn to_box(o: OutputJson) -> BoxData {
    BoxData {
        address: tree_to_address(&o.ergo_tree).unwrap_or_default(),
        box_id: o.box_id,
        value: o.value,
        tokens: o
            .assets
            .into_iter()
            .map(|a| Token {
                token_id: a.token_id,
                amount: a.amount,
            })
            .collect(),
    }
}

fn to_tx(t: TxJson, resolved: &HashMap<String, BoxData>) -> Tx {
    Tx {
        id: t.id,
        size: t.size.unwrap_or(0),
        inputs: t
            .inputs
            .into_iter()
            .map(|i| Input {
                resolved: resolved.get(&i.box_id).cloned(),
                box_id: i.box_id,
            })
            .collect(),
        outputs: t.outputs.into_iter().map(to_box).collect(),
        creation_ts_ms: None,
    }
}

#[derive(Clone)]
pub struct NodeClient {
    http: reqwest::Client,
    base: String,
}

impl NodeClient {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, p: &str) -> String {
        format!("{}{}", self.base, p)
    }

    pub async fn info(&self) -> Result<NodeInfo> {
        let i: InfoJson = get_json(&self.http, &self.url("/info")).await?;
        let indexed_height =
            get_json::<IndexedHeightJson>(&self.http, &self.url("/blockchain/indexedHeight"))
                .await
                .ok()
                .map(|h| h.indexed_height);
        Ok(NodeInfo {
            full_height: i.full_height.unwrap_or(0),
            headers_height: i.headers_height.unwrap_or(0),
            peers: i.peers_count.unwrap_or(0),
            app_version: i.app_version.unwrap_or_default(),
            max_block_size: i.parameters.map(|p| p.max_block_size).unwrap_or(0),
            indexed_height,
        })
    }

    pub async fn mempool_ids(&self) -> Result<Vec<TxId>> {
        get_json(
            &self.http,
            &self.url("/transactions/unconfirmed/transactionIds"),
        )
        .await
    }

    pub async fn mempool_txs(&self, ids: &[TxId]) -> Result<Vec<Tx>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let raw: Vec<TxJson> = match post_json(
            &self.http,
            &self.url("/transactions/unconfirmed/byTransactionIds"),
            ids,
        )
        .await
        {
            Ok(v) => v,
            Err(_) => {
                let mut v = Vec::new();
                for id in ids {
                    let url = self.url(&format!("/transactions/unconfirmed/byTransactionId/{id}"));
                    if let Ok(t) = get_json::<TxJson>(&self.http, &url).await {
                        v.push(t);
                    }
                }
                v
            }
        };
        let box_ids: Vec<String> = raw
            .iter()
            .flat_map(|t| t.inputs.iter().map(|i| i.box_id.clone()))
            .collect();
        let resolved = self.boxes(&box_ids).await;
        Ok(raw.into_iter().map(|t| to_tx(t, &resolved)).collect())
    }

    /// Input boxes from UTXO set + mempool. Failures leave inputs unresolved.
    async fn boxes(&self, ids: &[String]) -> HashMap<String, BoxData> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(BOX_CHUNK) {
            if let Ok(found) = post_json::<_, Vec<OutputJson>>(
                &self.http,
                &self.url("/utxo/withPool/byIds"),
                chunk,
            )
            .await
            {
                for o in found {
                    let b = to_box(o);
                    out.insert(b.box_id.clone(), b);
                }
            }
        }
        out
    }

    pub async fn last_headers(&self, n: u32) -> Result<Vec<BlockRef>> {
        let hs: Vec<HeaderJson> =
            get_json(&self.http, &self.url(&format!("/blocks/lastHeaders/{n}"))).await?;
        Ok(hs
            .into_iter()
            .map(|h| BlockRef {
                id: h.id,
                height: h.height,
                timestamp_ms: h.timestamp,
            })
            .collect())
    }

    pub async fn block(&self, header: &BlockRef) -> Result<Block> {
        let b: BlockTxsJson = get_json(
            &self.http,
            &self.url(&format!("/blocks/{}/transactions", header.id)),
        )
        .await?;
        let empty = HashMap::new();
        let txs: Vec<Tx> = b
            .transactions
            .into_iter()
            .map(|t| to_tx(t, &empty))
            .collect();
        let reward = txs.first().and_then(|t| find_miner_reward(&t.outputs));
        Ok(Block {
            id: header.id.clone(),
            height: header.height,
            timestamp_ms: header.timestamp_ms,
            size: b.size,
            miner_address: reward.map(|r| r.address.clone()),
            miner_reward: reward.map(|r| r.value).unwrap_or(0),
            tx_ids: txs.iter().map(|t| t.id.clone()).collect(),
        })
    }

    pub async fn token(&self, token_id: &str) -> Result<TokenMeta> {
        let t: TokenJson = get_json(
            &self.http,
            &self.url(&format!("/blockchain/token/byId/{token_id}")),
        )
        .await?;
        Ok(TokenMeta {
            token_id: t.id,
            name: t.name,
            decimals: t.decimals.unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ergotree::tree_to_address;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const INFO: &str = include_str!("../../tests/fixtures/node/info.json");
    const INDEXED: &str = include_str!("../../tests/fixtures/node/indexed_height.json");
    const IDS: &str = include_str!("../../tests/fixtures/node/mempool_ids.json");
    const TXS: &str = include_str!("../../tests/fixtures/node/mempool_txs.json");
    const TX_A: &str = include_str!("../../tests/fixtures/node/tx_a.json");
    const BOXES: &str = include_str!("../../tests/fixtures/node/boxes.json");
    const HEADERS: &str = include_str!("../../tests/fixtures/node/last_headers.json");
    const BLOCK: &str = include_str!("../../tests/fixtures/node/block_txs.json");
    const TOKEN: &str = include_str!("../../tests/fixtures/node/token.json");

    async fn mock(server: &MockServer, verb: &str, p: &str, status: u16, body: &str) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn info_includes_indexed_height_and_block_size() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/info", 200, INFO).await;
        mock(&s, "GET", "/blockchain/indexedHeight", 200, INDEXED).await;
        let info = NodeClient::new(http_client(), &s.uri())
            .info()
            .await
            .unwrap();
        assert_eq!(info.full_height, 1886101);
        assert_eq!(info.max_block_size, 1271009);
        assert_eq!(info.peers, 31);
        assert_eq!(info.app_version, "6.0.1");
        assert_eq!(info.indexed_height, Some(1886100));
    }

    #[tokio::test]
    async fn info_without_index_is_not_indexed() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/info", 200, INFO).await;
        mock(&s, "GET", "/blockchain/indexedHeight", 404, "{}").await;
        let info = NodeClient::new(http_client(), &s.uri())
            .info()
            .await
            .unwrap();
        assert_eq!(info.indexed_height, None);
    }

    #[tokio::test]
    async fn mempool_txs_resolve_inputs_and_addresses() {
        let s = MockServer::start().await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed/transactionIds",
            200,
            IDS,
        )
        .await;
        mock(
            &s,
            "POST",
            "/transactions/unconfirmed/byTransactionIds",
            200,
            TXS,
        )
        .await;
        mock(&s, "POST", "/utxo/withPool/byIds", 200, BOXES).await;
        let c = NodeClient::new(http_client(), &s.uri());
        let ids = c.mempool_ids().await.unwrap();
        assert_eq!(ids, vec!["tx-a", "tx-b"]);
        let txs = c.mempool_txs(&ids).await.unwrap();
        assert_eq!(txs.len(), 2);
        let a = &txs[0];
        assert_eq!(a.size, 412);
        assert_eq!(
            a.outputs[0].address,
            "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq"
        );
        assert_eq!(a.outputs[1].address, crate::metrics::FEE_ADDRESS);
        assert_eq!(a.outputs[0].tokens[0].token_id, "tok-1");
        let resolved = a.inputs[0].resolved.as_ref().expect("input resolved");
        assert_eq!(resolved.value, 11889000000);
        assert_eq!(
            resolved.address,
            tree_to_address(
                "0008cd021111111111111111111111111111111111111111111111111111111111111111"
            )
            .unwrap()
        );
        assert_eq!(txs[1].outputs[0].address, "4MQyMKvMbnCJG3aJ");
    }

    #[tokio::test]
    async fn mempool_txs_fall_back_to_single_gets() {
        let s = MockServer::start().await;
        mock(
            &s,
            "POST",
            "/transactions/unconfirmed/byTransactionIds",
            500,
            "",
        )
        .await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed/byTransactionId/tx-a",
            200,
            TX_A,
        )
        .await;
        mock(&s, "POST", "/utxo/withPool/byIds", 404, "{}").await;
        let c = NodeClient::new(http_client(), &s.uri());
        let txs = c.mempool_txs(&["tx-a".to_string()]).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id, "tx-a");
        assert!(
            txs[0].inputs[0].resolved.is_none(),
            "box lookup failed, input left unresolved"
        );
    }

    #[tokio::test]
    async fn empty_id_list_makes_no_requests() {
        let c = NodeClient::new(http_client(), "http://127.0.0.1:9");
        assert!(c.mempool_txs(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn block_has_tx_ids_and_miner() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/blocks/lastHeaders/2", 200, HEADERS).await;
        mock(&s, "GET", "/blocks/hdr-1/transactions", 200, BLOCK).await;
        let c = NodeClient::new(http_client(), &s.uri());
        let headers = c.last_headers(2).await.unwrap();
        assert_eq!(
            headers[1],
            BlockRef {
                id: "hdr-1".into(),
                height: 1886101,
                timestamp_ms: 1790978083213
            }
        );
        let b = c.block(&headers[1]).await.unwrap();
        assert_eq!(b.height, 1886101);
        assert_eq!(b.size, 187236);
        assert_eq!(b.tx_ids, vec!["cb-1", "tx-a"]);
        assert_eq!(
            b.miner_address.as_deref(),
            Some(
                "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY"
            )
        );
        assert_eq!(b.miner_reward, 12000000000);
    }

    #[tokio::test]
    async fn token_meta() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/blockchain/token/byId/tok-1", 200, TOKEN).await;
        let t = NodeClient::new(http_client(), &s.uri())
            .token("tok-1")
            .await
            .unwrap();
        assert_eq!(
            t,
            TokenMeta {
                token_id: "tok-1".into(),
                name: Some("SigUSD".into()),
                decimals: 2
            }
        );
    }
}
