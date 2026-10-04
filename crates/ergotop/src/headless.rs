//! `--headless`: print reconciled mempool activity as text lines.
use ergotop_core::classify::{BookEntry, Builtin, Classifier};
use ergotop_core::config::{cache_dir, AddressesFile, Config};
use ergotop_core::model::{nano_to_erg, SourceId};
use ergotop_core::reconcile::{Reconciler, SourceView, TxEntry, Update};
use ergotop_core::sources::runtime::{spawn_all, Timing};
use ergotop_core::sources::SourceEvent;

use crate::clock::now_ms;

pub fn tx_line(e: &TxEntry) -> String {
    let id: String = e.tx.id.chars().take(8).collect();
    format!(
        "+ {id} {:<16} fee {:.4} value {:.2}{} {} B",
        e.class.class.name,
        nano_to_erg(e.metrics.fee),
        nano_to_erg(e.metrics.value),
        if e.metrics.approx { "~" } else { "" },
        e.tx.size
    )
}

pub fn sources_line(views: &[SourceView], active: Option<&SourceId>) -> String {
    let parts: Vec<String> = views
        .iter()
        .map(|v| {
            format!(
                "{} {}{} {}ms {}tx",
                v.id,
                if v.status.usable() { "●" } else { "○" },
                if Some(&v.id) == active { "*" } else { "" },
                v.latency_ms
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "-".into()),
                v.ids.len()
            )
        })
        .collect();
    format!("sources: {}", parts.join(" | "))
}

fn describe(rec: &Reconciler, cls: &Classifier, u: &Update) -> Vec<String> {
    match u {
        Update::Added(ids) => ids
            .iter()
            .filter_map(|id| rec.pool().get(id))
            .map(tx_line)
            .collect(),
        Update::Mined { height, tx_ids } => vec![format!(
            "⛏ block {height}: {} mempool txs mined",
            tx_ids.len()
        )],
        Update::Dropped(ids) => vec![format!("- dropped {}: {}", ids.len(), ids.join(", "))],
        Update::Resynced => vec![format!(
            "= resynced on {}: {} txs",
            rec.active().map(|s| s.0.as_str()).unwrap_or("none"),
            rec.pool().len()
        )],
        Update::BlockAdded(h) => {
            let miner = rec
                .recent_blocks()
                .into_iter()
                .find(|b| b.height == *h)
                .and_then(|b| b.miner_address.as_deref())
                .and_then(|a| cls.lookup(a))
                .map(|c| c.name)
                .unwrap_or_else(|| "Other".into());
            vec![format!("# block {h} by {miner}")]
        }
        Update::SourcesChanged => vec![sources_line(rec.views(), rec.active())],
    }
}

pub async fn run(cfg: Config, addrs: AddressesFile) -> anyhow::Result<()> {
    let specs = cfg.sources();
    let (mut rx, _refresh) = spawn_all(&specs, Timing::default(), cache_dir());
    let builtin = Builtin::load();
    let mut book: Vec<BookEntry> = Vec::new();
    let mut cls = Classifier::new(&builtin, &book, &addrs.address);
    let mut rec = Reconciler::new(specs.iter().map(|s| (s.id.clone(), s.kind)).collect());

    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    SourceEvent::AddressBook(entries) => {
                        book = entries;
                        cls = Classifier::new(&builtin, &book, &addrs.address);
                        rec.reclassify(&cls);
                        println!("address book: {} entries", book.len());
                    }
                    SourceEvent::Price(p) => println!("price: ${p:.4}"),
                    SourceEvent::TokenMeta(_) => {}
                    other => {
                        for u in rec.apply(other, now_ms(), &cls) {
                            for line in describe(&rec, &cls, &u) {
                                println!("{line}");
                            }
                        }
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergotop_core::classify::{Classification, Kind, Rgb, TxClass};
    use ergotop_core::metrics::TxMetrics;
    use ergotop_core::model::{SourceKind, SourceStatus, Tx};
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn formats_tx_line() {
        let e = TxEntry {
            tx: Tx {
                id: "abcdef0123456789".into(),
                size: 412,
                inputs: vec![],
                outputs: vec![],
                creation_ts_ms: None,
            },
            first_seen_ms: 0,
            seen_by: BTreeSet::new(),
            class: TxClass {
                class: Classification {
                    name: "Spectrum".into(),
                    kind: Kind::Service,
                    color: Rgb(0, 0, 0),
                },
                from: None,
            },
            metrics: TxMetrics {
                fee: 1_500_000,
                value: 11_880_000_000,
                approx: true,
            },
        };
        assert_eq!(
            tx_line(&e),
            "+ abcdef01 Spectrum         fee 0.0015 value 11.88~ 412 B"
        );
    }

    #[test]
    fn formats_sources_line() {
        let view = |id: &str, status: SourceStatus, latency: Option<u64>, n: usize| SourceView {
            id: SourceId(id.into()),
            kind: SourceKind::Node,
            status,
            ids: (0..n).map(|i| i.to_string()).collect::<HashSet<_>>(),
            last_update_ms: None,
            latency_ms: latency,
            info: None,
        };
        let views = vec![
            view("node-a", SourceStatus::Up, Some(23), 2),
            view("p2p", SourceStatus::Down("x".into()), None, 0),
        ];
        assert_eq!(
            sources_line(&views, Some(&SourceId("node-a".into()))),
            "sources: node-a ●* 23ms 2tx | p2p ○ -ms 0tx"
        );
    }
}
