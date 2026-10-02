//! Folds per-source events into one canonical mempool.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::classify::{Classifier, TxClass};
use crate::metrics::{tx_metrics, TxMetrics};
use crate::model::{Block, NodeInfo, SourceId, SourceKind, SourceStatus, Tx, TxId};
use crate::sources::SourceEvent;

/// Active polls a removed tx waits for its block before it is declared dropped.
pub const DROP_GRACE_POLLS: u8 = 2;
const RECENT_BLOCKS: usize = 10;
const EMPTY_GUARD_MIN: usize = 5;

#[derive(Clone, Debug)]
pub struct TxEntry {
    pub tx: Tx,
    pub first_seen_ms: u64,
    pub seen_by: BTreeSet<SourceId>,
    pub class: TxClass,
    pub metrics: TxMetrics,
}

#[derive(Clone, Debug)]
pub struct SourceView {
    pub id: SourceId,
    pub kind: SourceKind,
    pub status: SourceStatus,
    pub ids: HashSet<TxId>,
    pub last_update_ms: Option<u64>,
    pub latency_ms: Option<u64>,
    pub info: Option<NodeInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    Added(Vec<TxId>),
    Mined { height: u32, tx_ids: Vec<TxId> },
    Dropped(Vec<TxId>),
    /// The pool was rebuilt (startup or failover); redraw without per-tx animation.
    Resynced,
    BlockAdded(u32),
    SourcesChanged,
}

pub struct Reconciler {
    views: Vec<SourceView>,
    active: Option<SourceId>,
    pool: HashMap<TxId, TxEntry>,
    bodies: HashMap<TxId, Tx>,
    first_seen: HashMap<TxId, u64>,
    pending: HashMap<TxId, u8>,
    blocks: VecDeque<Block>,
    empty_strike: bool,
}

impl Reconciler {
    pub fn new(sources: Vec<(SourceId, SourceKind)>) -> Self {
        let views = sources
            .into_iter()
            .map(|(id, kind)| SourceView {
                id,
                kind,
                status: SourceStatus::Unknown,
                ids: HashSet::new(),
                last_update_ms: None,
                latency_ms: None,
                info: None,
            })
            .collect();
        Reconciler {
            views,
            active: None,
            pool: HashMap::new(),
            bodies: HashMap::new(),
            first_seen: HashMap::new(),
            pending: HashMap::new(),
            blocks: VecDeque::new(),
            empty_strike: false,
        }
    }

    pub fn active(&self) -> Option<&SourceId> {
        self.active.as_ref()
    }

    pub fn pool(&self) -> &HashMap<TxId, TxEntry> {
        &self.pool
    }

    pub fn views(&self) -> &[SourceView] {
        &self.views
    }

    pub fn recent_blocks(&self) -> Vec<&Block> {
        let mut v: Vec<&Block> = self.blocks.iter().collect();
        v.sort_by(|a, b| b.height.cmp(&a.height));
        v
    }

    /// Ids that `source` has but no other source has.
    pub fn only_in(&self, source: &SourceId) -> Vec<TxId> {
        let Some(view) = self.views.iter().find(|v| &v.id == source) else { return vec![] };
        let mut out: Vec<TxId> = view
            .ids
            .iter()
            .filter(|id| self.views.iter().all(|o| &o.id == source || !o.ids.contains(*id)))
            .cloned()
            .collect();
        out.sort();
        out
    }

    pub fn reclassify(&mut self, cls: &Classifier) {
        for e in self.pool.values_mut() {
            e.class = cls.classify_tx(&e.tx);
        }
    }

    pub fn apply(&mut self, ev: SourceEvent, now_ms: u64, cls: &Classifier) -> Vec<Update> {
        match ev {
            SourceEvent::Status { source, status } => {
                let Some(v) = self.views.iter_mut().find(|v| v.id == source) else { return vec![] };
                if v.status == status {
                    return vec![];
                }
                v.status = status;
                let mut out = vec![Update::SourcesChanged];
                out.extend(self.reselect_active(cls));
                out
            }
            SourceEvent::Info { source, info } => match self.views.iter_mut().find(|v| v.id == source) {
                Some(v) => {
                    v.info = Some(info);
                    vec![Update::SourcesChanged]
                }
                None => vec![],
            },
            SourceEvent::Mempool { source, ids, new_txs, latency_ms } => {
                self.on_mempool(source, ids, new_txs, latency_ms, now_ms, cls)
            }
            SourceEvent::Block { block, .. } => self.on_block(block),
            SourceEvent::Price(_) | SourceEvent::AddressBook(_) | SourceEvent::TokenMeta(_) => vec![],
        }
    }

    fn on_mempool(
        &mut self,
        source: SourceId,
        ids: Vec<TxId>,
        new_txs: Vec<Tx>,
        latency_ms: u64,
        now_ms: u64,
        cls: &Classifier,
    ) -> Vec<Update> {
        let Some(idx) = self.views.iter().position(|v| v.id == source) else { return vec![] };
        for id in &ids {
            self.first_seen.entry(id.clone()).or_insert(now_ms);
        }
        for tx in new_txs {
            self.first_seen.entry(tx.id.clone()).or_insert(now_ms);
            self.bodies.entry(tx.id.clone()).or_insert(tx);
        }

        let is_active = self.active.as_ref() == Some(&source);
        if is_active && ids.is_empty() && self.pool.len() > EMPTY_GUARD_MIN && !self.empty_strike {
            self.empty_strike = true;
            return vec![];
        }
        if is_active {
            self.empty_strike = false;
        }

        let mut out = Vec::new();
        {
            let v = &mut self.views[idx];
            v.ids = ids.into_iter().collect();
            v.last_update_ms = Some(now_ms);
            v.latency_ms = Some(latency_ms);
            if !v.status.usable() {
                v.status = SourceStatus::Up;
                out.push(Update::SourcesChanged);
            }
        }
        let view_ids = &self.views[idx].ids;
        for e in self.pool.values_mut() {
            if view_ids.contains(&e.tx.id) {
                e.seen_by.insert(source.clone());
            } else {
                e.seen_by.remove(&source);
            }
        }

        let reselected = self.reselect_active(cls);
        if !reselected.is_empty() {
            out.extend(reselected);
        } else if self.active.as_ref() == Some(&source) {
            out.extend(self.diff_active(idx, cls));
        }
        self.prune();
        out
    }

    fn reselect_active(&mut self, cls: &Classifier) -> Vec<Update> {
        let next = self
            .views
            .iter()
            .find(|v| v.status.usable() && v.last_update_ms.is_some())
            .map(|v| v.id.clone());
        if next == self.active {
            return vec![];
        }
        self.active = next;
        self.resync(cls);
        vec![Update::Resynced]
    }

    fn resync(&mut self, cls: &Classifier) {
        self.pending.clear();
        self.empty_strike = false;
        let Some(idx) = self.active_idx() else {
            self.pool.clear();
            return;
        };
        let ids: Vec<TxId> = self.views[idx].ids.iter().cloned().collect();
        let mut pool = HashMap::new();
        for id in ids {
            if let Some(tx) = self.bodies.get(&id).cloned() {
                pool.insert(id, self.make_entry(tx, cls));
            }
        }
        self.pool = pool;
    }

    fn diff_active(&mut self, idx: usize, cls: &Classifier) -> Vec<Update> {
        let (mut added, removed): (Vec<TxId>, Vec<TxId>) = {
            let active_ids = &self.views[idx].ids;
            let added = active_ids
                .iter()
                .filter(|id| !self.pool.contains_key(*id) && self.bodies.contains_key(*id))
                .cloned()
                .collect();
            let removed = self.pool.keys().filter(|id| !active_ids.contains(*id)).cloned().collect();
            (added, removed)
        };
        for id in &added {
            self.pending.remove(id);
            let entry = self.make_entry(self.bodies[id].clone(), cls);
            self.pool.insert(id.clone(), entry);
        }

        let mut dropped = Vec::new();
        for (id, polls) in self.pending.iter_mut() {
            *polls += 1;
            if *polls >= DROP_GRACE_POLLS {
                dropped.push(id.clone());
            }
        }
        for id in &dropped {
            self.pending.remove(id);
        }

        let mut mined: BTreeMap<u32, Vec<TxId>> = BTreeMap::new();
        for id in removed {
            self.pool.remove(&id);
            match self.block_height_of(&id) {
                Some(h) => mined.entry(h).or_default().push(id),
                None => {
                    self.pending.insert(id, 0);
                }
            }
        }

        let mut out = Vec::new();
        if !added.is_empty() {
            added.sort();
            out.push(Update::Added(added));
        }
        for (height, mut tx_ids) in mined {
            tx_ids.sort();
            out.push(Update::Mined { height, tx_ids });
        }
        if !dropped.is_empty() {
            dropped.sort();
            out.push(Update::Dropped(dropped));
        }
        out
    }

    fn on_block(&mut self, block: Block) -> Vec<Update> {
        if self.blocks.iter().any(|b| b.id == block.id) {
            return vec![];
        }
        let height = block.height;
        let mut mined: Vec<TxId> = self
            .pending
            .keys()
            .filter(|id| block.tx_ids.contains(*id))
            .cloned()
            .collect();
        for id in &mined {
            self.pending.remove(id);
        }
        self.blocks.push_front(block);
        self.blocks.truncate(RECENT_BLOCKS);
        let mut out = vec![Update::BlockAdded(height)];
        if !mined.is_empty() {
            mined.sort();
            out.push(Update::Mined { height, tx_ids: mined });
        }
        out
    }

    fn active_idx(&self) -> Option<usize> {
        let active = self.active.as_ref()?;
        self.views.iter().position(|v| &v.id == active)
    }

    fn block_height_of(&self, id: &TxId) -> Option<u32> {
        self.blocks.iter().find(|b| b.tx_ids.contains(id)).map(|b| b.height)
    }

    fn make_entry(&self, tx: Tx, cls: &Classifier) -> TxEntry {
        let seen_by = self
            .views
            .iter()
            .filter(|v| v.ids.contains(&tx.id))
            .map(|v| v.id.clone())
            .collect();
        TxEntry {
            first_seen_ms: self.first_seen.get(&tx.id).copied().unwrap_or(0),
            seen_by,
            class: cls.classify_tx(&tx),
            metrics: tx_metrics(&tx),
            tx,
        }
    }

    /// Forget bodies and timestamps no source still reports.
    fn prune(&mut self) {
        let views = &self.views;
        let pool = &self.pool;
        let pending = &self.pending;
        let keep =
            |id: &TxId| pool.contains_key(id) || pending.contains_key(id) || views.iter().any(|v| v.ids.contains(id));
        self.bodies.retain(|id, _| keep(id));
        self.first_seen.retain(|id, _| keep(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Builtin;
    use crate::model::test_util::{bx, tx};

    const W: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";

    fn node() -> SourceId {
        SourceId("node".into())
    }
    fn expl() -> SourceId {
        SourceId("p2p".into())
    }
    fn cls() -> Classifier {
        Classifier::new(&Builtin::default(), &[], &[])
    }
    fn rec() -> Reconciler {
        Reconciler::new(vec![(node(), SourceKind::Node), (expl(), SourceKind::Explorer)])
    }
    fn t(id: &str) -> Tx {
        tx(id, 100, vec![bx(W, 10)], vec![bx(W, 9)])
    }
    fn mempool(source: SourceId, ids: &[&str], new: &[&str]) -> SourceEvent {
        SourceEvent::Mempool {
            source,
            ids: ids.iter().map(|s| s.to_string()).collect(),
            new_txs: new.iter().map(|s| t(s)).collect(),
            latency_ms: 10,
        }
    }
    fn block(id: &str, height: u32, txs: &[&str]) -> SourceEvent {
        SourceEvent::Block {
            source: node(),
            block: Block {
                id: id.into(),
                height,
                timestamp_ms: 0,
                size: 0,
                tx_ids: txs.iter().map(|s| s.to_string()).collect(),
                miner_address: None,
                miner_reward: 0,
            },
        }
    }
    fn ids(v: &[&str]) -> Vec<TxId> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn first_snapshot_resyncs_then_new_tx_is_added() {
        let (mut r, c) = (rec(), cls());
        let u = r.apply(mempool(node(), &["a"], &["a"]), 1, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&node()));
        assert_eq!(r.pool().len(), 1);
        let u = r.apply(mempool(node(), &["a", "b"], &["b"]), 2, &c);
        assert_eq!(u, vec![Update::Added(ids(&["b"]))]);
        assert_eq!(r.pool()["b"].first_seen_ms, 2);
    }

    #[test]
    fn removal_after_block_is_mined() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(block("h1", 100, &["a"]), 2, &c), vec![Update::BlockAdded(100)]);
        let u = r.apply(mempool(node(), &["b"], &[]), 3, &c);
        assert_eq!(u, vec![Update::Mined { height: 100, tx_ids: ids(&["a"]) }]);
    }

    #[test]
    fn block_after_removal_is_still_mined() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 2, &c), vec![]);
        let u = r.apply(block("h1", 100, &["a"]), 3, &c);
        assert_eq!(u, vec![Update::BlockAdded(100), Update::Mined { height: 100, tx_ids: ids(&["a"]) }]);
    }

    #[test]
    fn removal_without_block_is_dropped_after_grace() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 2, &c), vec![]);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 3, &c), vec![]);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 4, &c), vec![Update::Dropped(ids(&["a"]))]);
    }

    #[test]
    fn duplicate_block_from_second_source_is_ignored() {
        let (mut r, c) = (rec(), cls());
        r.apply(block("h1", 100, &[]), 1, &c);
        assert_eq!(r.apply(block("h1", 100, &[]), 2, &c), vec![]);
        assert_eq!(r.recent_blocks().len(), 1);
    }

    #[test]
    fn single_empty_snapshot_is_ignored() {
        let (mut r, c) = (rec(), cls());
        let all = ["a", "b", "c", "d", "e", "f"];
        r.apply(mempool(node(), &all, &all), 1, &c);
        assert_eq!(r.apply(mempool(node(), &[], &[]), 2, &c), vec![]);
        assert_eq!(r.pool().len(), 6);
        r.apply(mempool(node(), &[], &[]), 3, &c);
        assert_eq!(r.pool().len(), 0, "second empty snapshot is believed");
    }

    #[test]
    fn failover_to_explorer_is_one_resync() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        r.apply(mempool(expl(), &["b", "c"], &["b", "c"]), 2, &c);
        assert_eq!(r.active(), Some(&node()));
        let u = r.apply(SourceEvent::Status { source: node(), status: SourceStatus::Down("timeout".into()) }, 3, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&expl()));
        let mut pool: Vec<&String> = r.pool().keys().collect();
        pool.sort();
        assert_eq!(pool, vec!["b", "c"]);
    }

    #[test]
    fn node_takes_over_from_explorer_when_it_reports() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(expl(), &["a"], &["a"]), 1, &c);
        assert_eq!(r.active(), Some(&expl()));
        let u = r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 2, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&node()));
        assert_eq!(r.pool().len(), 2);
    }

    #[test]
    fn tracks_seen_by_and_only_in() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        r.apply(mempool(expl(), &["b", "c"], &["b", "c"]), 2, &c);
        assert_eq!(r.pool()["b"].seen_by.len(), 2);
        assert_eq!(r.pool()["a"].seen_by.len(), 1);
        assert_eq!(r.only_in(&node()), ids(&["a"]));
        assert_eq!(r.only_in(&expl()), ids(&["c"]));
    }

    #[test]
    fn reclassify_updates_entries() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a"], &["a"]), 1, &c);
        assert_eq!(r.pool()["a"].class.class.name, "P2P");
        let book = [crate::classify::BookEntry { address: W.into(), name: "Kucoin".into(), kind: crate::classify::Kind::Exchange }];
        r.reclassify(&Classifier::new(&Builtin::default(), &book, &[]));
        assert_eq!(r.pool()["a"].class.class.name, "Kucoin");
    }
}
