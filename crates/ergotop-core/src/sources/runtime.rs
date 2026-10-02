//! Per-source polling tasks feeding one event channel.
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use super::addressbook;
use super::explorer::ExplorerClient;
use super::node::NodeClient;
use super::price;
use super::{http_client, SourceEvent};
use crate::config::SourceSpec;
use crate::model::{NodeInfo, SourceId, SourceKind, SourceStatus, TxId};

const MAX_INDEX_LAG: u32 = 2;
const FAILS_BEFORE_DOWN: u32 = 2;
const BLOCKS_PER_POLL: u32 = 3;
const BOOK_REFRESH: Duration = Duration::from_secs(24 * 3600);
const BOOK_RETRY: Duration = Duration::from_secs(10 * 60);
const PRICE_EVERY: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub node_mempool: Duration,
    pub node_info: Duration,
    pub node_headers: Duration,
    pub explorer_mempool: Duration,
    pub explorer_blocks: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            node_mempool: Duration::from_secs(1),
            node_info: Duration::from_secs(10),
            node_headers: Duration::from_secs(5),
            explorer_mempool: Duration::from_secs(5),
            explorer_blocks: Duration::from_secs(10),
        }
    }
}

pub fn backoff(fails: u32) -> Duration {
    Duration::from_secs((1u64 << fails.min(5)).min(30))
}

pub fn node_status(info: &NodeInfo) -> SourceStatus {
    match info.indexed_height {
        Some(ih) if info.full_height.saturating_sub(ih) > MAX_INDEX_LAG => {
            SourceStatus::Degraded(format!("index lag {}", info.full_height - ih))
        }
        _ => SourceStatus::Up,
    }
}

async fn send(tx: &mpsc::Sender<SourceEvent>, ev: SourceEvent) {
    let _ = tx.send(ev).await;
}

/// Marks a source Down only after `FAILS_BEFORE_DOWN` consecutive failures, so a single
/// timeout or 5xx does not trigger a failover (and a resync) of the whole pool.
async fn report_failure(
    tx: &mpsc::Sender<SourceEvent>,
    id: &SourceId,
    fails: u32,
    e: &super::SourceError,
) {
    if fails >= FAILS_BEFORE_DOWN {
        send(
            tx,
            SourceEvent::Status {
                source: id.clone(),
                status: SourceStatus::Down(e.to_string()),
            },
        )
        .await;
    }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

pub async fn run_node(
    id: SourceId,
    client: NodeClient,
    timing: Timing,
    tx: mpsc::Sender<SourceEvent>,
) {
    let mut known: HashSet<TxId> = HashSet::new();
    let mut seen_tokens: HashSet<String> = HashSet::new();
    let (token_tx, token_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(run_tokens(client.clone(), token_rx, tx.clone()));
    let mut last_height: u32 = 0;
    let mut fails: u32 = 0;
    let mut indexed = false;
    let mut next_info = Instant::now();
    let mut next_headers = Instant::now();

    while !tx.is_closed() {
        if Instant::now() >= next_info {
            match client.info().await {
                Ok(info) => {
                    indexed = info.indexed_height.is_some();
                    send(
                        &tx,
                        SourceEvent::Status {
                            source: id.clone(),
                            status: node_status(&info),
                        },
                    )
                    .await;
                    send(
                        &tx,
                        SourceEvent::Info {
                            source: id.clone(),
                            info,
                        },
                    )
                    .await;
                    next_info = Instant::now() + timing.node_info;
                }
                Err(e) => {
                    fails += 1;
                    report_failure(&tx, &id, fails, &e).await;
                    sleep(backoff(fails)).await;
                    continue;
                }
            }
        }

        let started = Instant::now();
        let polled = match client.mempool_ids().await {
            Ok(ids) => {
                let new_ids: Vec<TxId> = ids
                    .iter()
                    .filter(|i| !known.contains(*i))
                    .cloned()
                    .collect();
                client
                    .mempool_txs(&new_ids)
                    .await
                    .map(|new_txs| (ids, new_ids, new_txs))
            }
            Err(e) => Err(e),
        };
        match polled {
            Ok((ids, new_ids, new_txs)) => {
                fails = 0;
                let returned: HashSet<&TxId> = new_txs.iter().map(|t| &t.id).collect();
                let missing: HashSet<&TxId> =
                    new_ids.iter().filter(|i| !returned.contains(i)).collect();
                known = ids
                    .iter()
                    .filter(|i| !missing.contains(i))
                    .cloned()
                    .collect();
                if indexed {
                    for t in &new_txs {
                        for tok in t.outputs.iter().flat_map(|o| o.tokens.iter()) {
                            if seen_tokens.insert(tok.token_id.clone()) {
                                let _ = token_tx.send(tok.token_id.clone());
                            }
                        }
                    }
                }
                let latency_ms = ms(started.elapsed());
                send(
                    &tx,
                    SourceEvent::Mempool {
                        source: id.clone(),
                        ids,
                        new_txs,
                        latency_ms,
                    },
                )
                .await;
            }
            Err(e) => {
                fails += 1;
                report_failure(&tx, &id, fails, &e).await;
                next_info = Instant::now();
                sleep(backoff(fails)).await;
                continue;
            }
        }

        if Instant::now() >= next_headers {
            if let Ok(mut headers) = client.last_headers(BLOCKS_PER_POLL).await {
                headers.retain(|h| h.height > last_height);
                headers.sort_by_key(|h| h.height);
                for h in headers {
                    if let Ok(block) = client.block(&h).await {
                        last_height = h.height;
                        send(
                            &tx,
                            SourceEvent::Block {
                                source: id.clone(),
                                block,
                            },
                        )
                        .await;
                    }
                }
            }
            next_headers = Instant::now() + timing.node_headers;
        }

        sleep(timing.node_mempool).await;
    }
}

/// Fetches token metadata off the node's poll path; ends when `run_node` drops its sender.
async fn run_tokens(
    client: NodeClient,
    mut ids: mpsc::UnboundedReceiver<String>,
    tx: mpsc::Sender<SourceEvent>,
) {
    while let Some(token_id) = ids.recv().await {
        if let Ok(meta) = client.token(&token_id).await {
            send(&tx, SourceEvent::TokenMeta(meta)).await;
        }
    }
}

pub async fn run_explorer(
    id: SourceId,
    client: ExplorerClient,
    timing: Timing,
    tx: mpsc::Sender<SourceEvent>,
) {
    let mut known: HashSet<TxId> = HashSet::new();
    let mut last_height: u32 = 0;
    let mut fails: u32 = 0;
    let mut next_blocks = Instant::now();

    while !tx.is_closed() {
        let started = Instant::now();
        match client.mempool().await {
            Ok(txs) => {
                fails = 0;
                let ids: Vec<TxId> = txs.iter().map(|t| t.id.clone()).collect();
                let new_txs = txs.into_iter().filter(|t| !known.contains(&t.id)).collect();
                known = ids.iter().cloned().collect();
                let latency_ms = ms(started.elapsed());
                send(
                    &tx,
                    SourceEvent::Mempool {
                        source: id.clone(),
                        ids,
                        new_txs,
                        latency_ms,
                    },
                )
                .await;
            }
            Err(e) => {
                fails += 1;
                report_failure(&tx, &id, fails, &e).await;
                sleep(backoff(fails)).await;
                continue;
            }
        }

        if Instant::now() >= next_blocks {
            if let Ok(mut refs) = client.latest_blocks(BLOCKS_PER_POLL).await {
                refs.retain(|r| r.height > last_height);
                refs.sort_by_key(|r| r.height);
                for r in refs {
                    if let Ok(block) = client.block(&r.id).await {
                        last_height = r.height;
                        send(
                            &tx,
                            SourceEvent::Block {
                                source: id.clone(),
                                block,
                            },
                        )
                        .await;
                    }
                }
            }
            next_blocks = Instant::now() + timing.explorer_blocks;
        }

        sleep(timing.explorer_mempool).await;
    }
}

pub async fn run_address_book(
    http: reqwest::Client,
    base: String,
    cache: Option<PathBuf>,
    tx: mpsc::Sender<SourceEvent>,
) {
    let (entries, mut refresh) = addressbook::initial(cache.as_deref());
    send(&tx, SourceEvent::AddressBook(entries)).await;
    while !tx.is_closed() {
        let mut ok = true;
        if refresh {
            match addressbook::fetch(&http, &base).await {
                Ok((raw, entries)) => {
                    if let Some(path) = &cache {
                        let _ = addressbook::save_cache(path, &raw);
                    }
                    send(&tx, SourceEvent::AddressBook(entries)).await;
                }
                Err(e) => {
                    tracing::warn!("address book refresh failed: {e}");
                    ok = false;
                }
            }
        }
        sleep(next_book_wait(ok)).await;
        refresh = true;
    }
}

/// 24h after a good refresh; a failed one is retried after `BOOK_RETRY`.
fn next_book_wait(fetched_ok: bool) -> Duration {
    if fetched_ok {
        BOOK_REFRESH
    } else {
        BOOK_RETRY
    }
}

pub async fn run_price(http: reqwest::Client, tx: mpsc::Sender<SourceEvent>) {
    while !tx.is_closed() {
        if let Ok(p) = price::fetch_price(&http, price::PRICE_URL).await {
            send(&tx, SourceEvent::Price(p)).await;
        }
        sleep(PRICE_EVERY).await;
    }
}

/// Spawns one task per source plus address book and price. Call inside a tokio runtime.
pub fn spawn_all(
    specs: &[SourceSpec],
    timing: Timing,
    cache_dir: Option<PathBuf>,
) -> mpsc::Receiver<SourceEvent> {
    let (tx, rx) = mpsc::channel(1024);
    let http = http_client();
    for s in specs {
        match s.kind {
            SourceKind::Node => {
                tokio::spawn(run_node(
                    s.id.clone(),
                    NodeClient::new(http.clone(), &s.url),
                    timing,
                    tx.clone(),
                ));
            }
            SourceKind::Explorer => {
                tokio::spawn(run_explorer(
                    s.id.clone(),
                    ExplorerClient::new(http.clone(), &s.url),
                    timing,
                    tx.clone(),
                ));
            }
        }
    }
    let cache = addressbook::default_cache_path(cache_dir);
    tokio::spawn(run_address_book(
        http.clone(),
        addressbook::BOOK_API.to_string(),
        cache,
        tx.clone(),
    ));
    tokio::spawn(run_price(http, tx));
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast() -> Timing {
        let ms = Duration::from_millis(50);
        Timing {
            node_mempool: ms,
            node_info: ms,
            node_headers: ms,
            explorer_mempool: ms,
            explorer_blocks: ms,
        }
    }

    async fn mock(server: &MockServer, verb: &str, p: &str, status: u16, body: &str) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    /// Collects events until every wanted event kind has arrived or 10s pass.
    async fn collect(
        rx: &mut mpsc::Receiver<SourceEvent>,
        mut want: Vec<&'static str>,
    ) -> Vec<SourceEvent> {
        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !want.is_empty() {
            let ev = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .expect("timed out")
                .expect("closed");
            let kind = match &ev {
                SourceEvent::Status { .. } => "status",
                SourceEvent::Mempool { .. } => "mempool",
                SourceEvent::Block { .. } => "block",
                SourceEvent::Info { .. } => "info",
                SourceEvent::TokenMeta(_) => "token",
                SourceEvent::AddressBook(_) => "book",
                SourceEvent::Price(_) => "price",
            };
            want.retain(|w| *w != kind);
            got.push(ev);
        }
        got
    }

    #[test]
    fn backoff_doubles_to_cap() {
        let secs: Vec<u64> = (1..=7).map(|f| backoff(f).as_secs()).collect();
        assert_eq!(secs, vec![2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff(0).as_secs(), 1);
    }

    #[test]
    fn index_lag_degrades() {
        let mut info = NodeInfo {
            full_height: 100,
            indexed_height: Some(99),
            ..Default::default()
        };
        assert_eq!(node_status(&info), SourceStatus::Up);
        info.indexed_height = Some(90);
        assert_eq!(
            node_status(&info),
            SourceStatus::Degraded("index lag 10".into())
        );
        info.indexed_height = None;
        assert_eq!(node_status(&info), SourceStatus::Up);
    }

    #[tokio::test]
    async fn node_loop_emits_info_mempool_block_and_tokens() {
        let s = MockServer::start().await;
        mock(
            &s,
            "GET",
            "/info",
            200,
            include_str!("../../tests/fixtures/node/info.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blockchain/indexedHeight",
            200,
            include_str!("../../tests/fixtures/node/indexed_height.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed/transactionIds",
            200,
            include_str!("../../tests/fixtures/node/mempool_ids.json"),
        )
        .await;
        mock(
            &s,
            "POST",
            "/transactions/unconfirmed/byTransactionIds",
            200,
            include_str!("../../tests/fixtures/node/mempool_txs.json"),
        )
        .await;
        mock(
            &s,
            "POST",
            "/utxo/withPool/byIds",
            200,
            include_str!("../../tests/fixtures/node/boxes.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blocks/lastHeaders/3",
            200,
            include_str!("../../tests/fixtures/node/last_headers.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blocks/hdr-0/transactions",
            200,
            include_str!("../../tests/fixtures/node/block_txs.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blocks/hdr-1/transactions",
            200,
            include_str!("../../tests/fixtures/node/block_txs.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blockchain/token/byId/tok-1",
            200,
            include_str!("../../tests/fixtures/node/token.json"),
        )
        .await;

        let (tx, mut rx) = mpsc::channel(64);
        let id = SourceId("node-a".into());
        let handle = tokio::spawn(run_node(
            id.clone(),
            NodeClient::new(http_client(), &s.uri()),
            fast(),
            tx,
        ));
        let events = collect(&mut rx, vec!["status", "info", "mempool", "block", "token"]).await;
        handle.abort();

        let mempool = events.iter().find_map(|e| match e {
            SourceEvent::Mempool {
                source,
                ids,
                new_txs,
                ..
            } => Some((source, ids, new_txs)),
            _ => None,
        });
        let (source, ids, new_txs) = mempool.unwrap();
        assert_eq!(source, &id);
        assert_eq!(ids, &vec!["tx-a".to_string(), "tx-b".to_string()]);
        assert_eq!(new_txs.len(), 2);
        assert!(events.iter().any(|e| matches!(
            e,
            SourceEvent::Status {
                status: SourceStatus::Up,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn node_loop_reports_down_for_unreachable_node() {
        let (tx, mut rx) = mpsc::channel(8);
        let client = NodeClient::new(http_client(), "http://127.0.0.1:9");
        let handle = tokio::spawn(run_node(SourceId("dead".into()), client, fast(), tx));
        let events = collect(&mut rx, vec!["status"]).await;
        handle.abort();
        assert!(matches!(
            &events[0],
            SourceEvent::Status {
                status: SourceStatus::Down(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn explorer_loop_emits_mempool_once_per_new_tx_and_blocks() {
        let s = MockServer::start().await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed",
            200,
            include_str!("../../tests/fixtures/explorer/unconfirmed.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/api/v1/blocks",
            200,
            include_str!("../../tests/fixtures/explorer/blocks.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/api/v1/blocks/blk-1",
            200,
            include_str!("../../tests/fixtures/explorer/block.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/api/v1/blocks/blk-2",
            200,
            include_str!("../../tests/fixtures/explorer/block.json"),
        )
        .await;

        let (tx, mut rx) = mpsc::channel(64);
        let handle = tokio::spawn(run_explorer(
            SourceId("p2p".into()),
            ExplorerClient::new(http_client(), &s.uri()),
            fast(),
            tx,
        ));
        let first = collect(&mut rx, vec!["mempool", "block"]).await;
        let second = collect(&mut rx, vec!["mempool"]).await;
        handle.abort();
        let new_counts: Vec<usize> = first
            .iter()
            .chain(second.iter())
            .filter_map(|e| match e {
                SourceEvent::Mempool { new_txs, .. } => Some(new_txs.len()),
                _ => None,
            })
            .collect();
        assert_eq!(new_counts[0], 1);
        assert_eq!(
            *new_counts.last().unwrap(),
            0,
            "already-sent bodies are not resent"
        );
    }

    #[tokio::test]
    async fn address_book_loop_sends_snapshot_then_fetched_entries() {
        let s = MockServer::start().await;
        let body = r#"{"items":[{"address":"9f","name":"X","type":"Exchange"}],"total":1}"#;
        mock(&s, "GET", "/addressbook/getAddresses", 200, body).await;
        let cache = std::env::temp_dir()
            .join(format!("ergotop-rt-{}", std::process::id()))
            .join("addressbook.json");
        let _ = std::fs::remove_file(&cache);
        let (tx, mut rx) = mpsc::channel(8);
        let handle = tokio::spawn(run_address_book(
            http_client(),
            s.uri(),
            Some(cache.clone()),
            tx,
        ));
        let first = collect(&mut rx, vec!["book"]).await;
        let second = collect(&mut rx, vec!["book"]).await;
        handle.abort();
        assert!(matches!(&first[0], SourceEvent::AddressBook(e) if e.len() >= 300));
        assert!(matches!(&second[0], SourceEvent::AddressBook(e) if e.len() == 1));
        assert!(cache.exists(), "fetched book is cached");
    }

    #[tokio::test]
    async fn single_explorer_failure_does_not_report_down() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/transactions/unconfirmed"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&s)
            .await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed",
            200,
            include_str!("../../tests/fixtures/explorer/unconfirmed.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/api/v1/blocks",
            200,
            include_str!("../../tests/fixtures/explorer/blocks.json"),
        )
        .await;

        let (tx, mut rx) = mpsc::channel(64);
        let handle = tokio::spawn(run_explorer(
            SourceId("p2p".into()),
            ExplorerClient::new(http_client(), &s.uri()),
            fast(),
            tx,
        ));
        let events = collect(&mut rx, vec!["mempool"]).await;
        handle.abort();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                SourceEvent::Status {
                    status: SourceStatus::Down(_),
                    ..
                }
            )),
            "one transient failure must not mark the source down: {events:?}"
        );
    }

    #[tokio::test]
    async fn repeated_failures_report_down() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/transactions/unconfirmed", 503, "").await;
        let (tx, mut rx) = mpsc::channel(8);
        let handle = tokio::spawn(run_explorer(
            SourceId("p2p".into()),
            ExplorerClient::new(http_client(), &s.uri()),
            fast(),
            tx,
        ));
        let events = collect(&mut rx, vec!["status"]).await;
        handle.abort();
        assert!(matches!(
            &events[0],
            SourceEvent::Status {
                status: SourceStatus::Down(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn slow_token_endpoint_does_not_stall_mempool_polling() {
        let s = MockServer::start().await;
        mock(
            &s,
            "GET",
            "/info",
            200,
            include_str!("../../tests/fixtures/node/info.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/blockchain/indexedHeight",
            200,
            include_str!("../../tests/fixtures/node/indexed_height.json"),
        )
        .await;
        mock(
            &s,
            "GET",
            "/transactions/unconfirmed/transactionIds",
            200,
            include_str!("../../tests/fixtures/node/mempool_ids.json"),
        )
        .await;
        mock(
            &s,
            "POST",
            "/transactions/unconfirmed/byTransactionIds",
            200,
            include_str!("../../tests/fixtures/node/mempool_txs.json"),
        )
        .await;
        mock(
            &s,
            "POST",
            "/utxo/withPool/byIds",
            200,
            include_str!("../../tests/fixtures/node/boxes.json"),
        )
        .await;
        mock(&s, "GET", "/blocks/lastHeaders/3", 200, "[]").await;
        Mock::given(method("GET"))
            .and(path("/blockchain/token/byId/tok-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(include_str!("../../tests/fixtures/node/token.json"))
                    .set_delay(Duration::from_millis(2500)),
            )
            .mount(&s)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let handle = tokio::spawn(run_node(
            SourceId("n".into()),
            NodeClient::new(http_client(), &s.uri()),
            fast(),
            tx,
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
        let mut mempools = 0;
        while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if matches!(ev, SourceEvent::Mempool { .. }) {
                mempools += 1;
            }
        }
        handle.abort();
        assert!(
            mempools >= 3,
            "only {mempools} mempool polls in 2s while a token fetch hung"
        );
    }

    #[test]
    fn failed_book_refresh_retries_within_minutes() {
        assert_eq!(next_book_wait(true), Duration::from_secs(24 * 3600));
        assert!(next_book_wait(false) <= Duration::from_secs(15 * 60));
    }
}
