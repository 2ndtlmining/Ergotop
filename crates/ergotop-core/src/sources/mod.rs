//! Data sources: node, explorers, address book, price, and their poll loops.
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod addressbook;
pub mod explorer;
pub mod node;
pub mod price;
pub mod runtime;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("http status {0}")]
    Status(u16),
    #[error("parse: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, SourceError>;

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent(concat!("ergotop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    if !resp.status().is_success() {
        return Err(SourceError::Status(resp.status().as_u16()));
    }
    let text = resp.text().await?;
    serde_json::from_str(&text).map_err(|e| SourceError::Parse(e.to_string()))
}

pub(crate) async fn get_json<T: DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T> {
    read_json(http.get(url).send().await?).await
}

pub(crate) async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    body: &B,
) -> Result<T> {
    read_json(http.post(url).json(body).send().await?).await
}

use crate::classify::BookEntry;
use crate::model::{Block, NodeInfo, SourceId, SourceStatus, TokenMeta, Tx, TxId};

#[derive(Clone, Debug)]
pub enum SourceEvent {
    Status {
        source: SourceId,
        status: SourceStatus,
    },
    /// `ids` is the full current mempool of `source`; `new_txs` are bodies not sent before.
    Mempool {
        source: SourceId,
        ids: Vec<TxId>,
        new_txs: Vec<Tx>,
        latency_ms: u64,
    },
    Block {
        source: SourceId,
        block: Block,
    },
    Info {
        source: SourceId,
        info: NodeInfo,
    },
    Price(f64),
    AddressBook(Vec<BookEntry>),
    TokenMeta(TokenMeta),
}
