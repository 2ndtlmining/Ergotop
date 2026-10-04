//! Pure application state: folds source events and key presses; no terminal I/O.
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ergotop_core::classify::{BookEntry, Builtin, Classifier};
use ergotop_core::config::{AddressesFile, LocalAddress, SourceSpec, UiConfig, UiState};
use ergotop_core::metrics::{rate_stats, RateStats, FEE_ADDRESS};
use ergotop_core::model::{
    nano_to_erg, Block, NodeInfo, SourceId, SourceKind, Token, TokenMeta, TxId,
};
use ergotop_core::packing::Shape;
use ergotop_core::reconcile::{Reconciler, TxEntry, Update};
use ergotop_core::sources::SourceEvent;

use crate::format;
use crate::theme::{rgb, Theme};
use crate::viz::{Visualizer, VizItem};

/// Ergo `maxBlockSize` (explorer /api/v1/epochs/params, 2026-10-03); used until a node reports it.
pub const DEFAULT_MAX_BLOCK_SIZE: u32 = 1_271_009;
const STATUS_MS: u64 = 3_000;
const BLOCK_FLASH_MS: u64 = 1_500;
const PAGE: usize = 10;
pub const EXPLORER_TX_URL: &str = "https://explorer.ergoplatform.com/en/transactions/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Packing,
    Sources,
}

impl View {
    pub fn parse(s: &str) -> View {
        match s {
            "dashboard" => View::Dashboard,
            "sources" => View::Sources,
            _ => View::Packing,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            View::Dashboard => "dashboard",
            View::Packing => "packing",
            View::Sources => "sources",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Rate,
    Fee,
    Value,
    Size,
    Age,
    Origin,
}

impl SortKey {
    pub fn next(self) -> SortKey {
        match self {
            SortKey::Rate => SortKey::Fee,
            SortKey::Fee => SortKey::Value,
            SortKey::Value => SortKey::Size,
            SortKey::Size => SortKey::Age,
            SortKey::Age => SortKey::Origin,
            SortKey::Origin => SortKey::Rate,
        }
    }

    pub const ALL: [SortKey; 6] = [
        SortKey::Rate,
        SortKey::Fee,
        SortKey::Value,
        SortKey::Size,
        SortKey::Age,
        SortKey::Origin,
    ];

    /// The key with this label; unknown labels fall back to fee rate.
    pub fn parse(s: &str) -> SortKey {
        SortKey::ALL
            .into_iter()
            .find(|k| k.label() == s)
            .unwrap_or(SortKey::Rate)
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Rate => "rate",
            SortKey::Fee => "fee",
            SortKey::Value => "value",
            SortKey::Size => "size",
            SortKey::Age => "age",
            SortKey::Origin => "origin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Detail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Copy(String),
    Open(String),
    /// Ask every source to poll now.
    Refresh,
}

/// Data older than this (ms) from the active source counts as stale.
const STALE_NODE_MS: u64 = 10_000;
const STALE_EXPLORER_MS: u64 = 30_000;

/// How current the shown mempool is; ages are in ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Connecting,
    Live(u64),
    Stale(u64),
    /// No usable source; the age of the last pool we got, if any.
    Offline(Option<u64>),
}

pub struct App {
    pub rec: Reconciler,
    pub cls: Classifier,
    builtin: Builtin,
    book: Vec<BookEntry>,
    local: Vec<LocalAddress>,
    pub tokens: HashMap<String, TokenMeta>,
    pub price: Option<f64>,
    /// Txs moving at least this many nanoERG are highlighted; 0 disables.
    pub whale_nano: u64,
    pub view: View,
    pub overlay: Overlay,
    pub filter: String,
    pub filtering: bool,
    pub sort: SortKey,
    /// Sort opposite to the key's natural direction (`S`).
    pub sort_reversed: bool,
    /// First visible line of the detail popup; clamped to its content when drawn.
    pub detail_scroll: u16,
    pub selected: usize,
    /// The selected tx; `selected` is re-derived from it whenever rows change.
    selected_id: Option<TxId>,
    pub source_sel: usize,
    /// Base URL per source, for copy/open in the Sources view.
    source_urls: HashMap<SourceId, String>,
    pub show_only: bool,
    pub theme: Theme,
    pub viz: Visualizer,
    /// Txs that left the pool but still occupy their slot until mined, dropped or expired.
    leaving: HashMap<TxId, VizItem>,
    pub status: Option<(String, u64)>,
    pub block_flash_until: u64,
    pub banner: Option<String>,
    /// Latest time seen from events, keys or ticks (ms); stamps visualizer relayouts.
    clock_ms: u64,
    /// When the active source last delivered the mempool (ms).
    last_pool_ms: Option<u64>,
    /// Second of the last tick, so ages and freshness redraw once a second.
    last_tick_sec: u64,
    /// Bumped by every event that can change the pool; keys derived-data caches.
    data_version: u64,
    stats_cache: RefCell<Option<StatsCacheEntry>>,
}

/// (data version, max block size) → fee-rate stats computed for them.
type StatsCacheEntry = ((u64, u32), Option<RateStats>);

/// Sorts by `key` (computed once per row), then tx id so the order is total and stable.
fn sort_keyed<'a, K: Ord>(v: &mut Vec<&'a TxEntry>, key: impl Fn(&'a TxEntry) -> K) {
    let mut keyed: Vec<(K, &'a TxEntry)> = v.drain(..).map(|e| (key(e), e)).collect();
    keyed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.tx.id.cmp(&b.1.tx.id)));
    v.extend(keyed.into_iter().map(|(_, e)| e));
}

fn matches_filter(e: &TxEntry, filter: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if let Some(n) = f
        .strip_prefix('>')
        .and_then(|s| s.trim().parse::<f64>().ok())
    {
        return nano_to_erg(e.metrics.value) >= n;
    }
    if let Some(n) = f
        .strip_prefix('<')
        .and_then(|s| s.trim().parse::<f64>().ok())
    {
        return nano_to_erg(e.metrics.value) <= n;
    }
    let f = f.to_lowercase();
    e.class.class.name.to_lowercase().contains(&f)
        || e.class.class.kind.label().to_lowercase().contains(&f)
        || e.class
            .from
            .as_deref()
            .is_some_and(|x| x.to_lowercase().contains(&f))
        || e.tx.id.starts_with(&f)
}

impl App {
    pub fn new(specs: &[SourceSpec], addrs: AddressesFile, ui: &UiConfig) -> App {
        let builtin = Builtin::load();
        let cls = Classifier::new(&builtin, &[], &addrs.address);
        let mut app = App {
            rec: Reconciler::new(specs.iter().map(|s| (s.id.clone(), s.kind)).collect()),
            cls,
            builtin,
            book: Vec::new(),
            local: addrs.address,
            tokens: HashMap::new(),
            price: None,
            whale_nano: (ui.whale_erg.max(0.0) * 1e9) as u64,
            view: View::parse(&ui.start_view),
            overlay: Overlay::None,
            filter: String::new(),
            filtering: false,
            sort: SortKey::parse(&ui.sort),
            sort_reversed: ui.sort_reversed,
            detail_scroll: 0,
            selected: 0,
            selected_id: None,
            source_sel: 0,
            source_urls: specs
                .iter()
                .map(|s| (s.id.clone(), s.url.clone()))
                .collect(),
            show_only: false,
            theme: Theme::by_name(&ui.theme),
            viz: Visualizer::new(),
            leaving: HashMap::new(),
            status: None,
            block_flash_until: 0,
            banner: None,
            clock_ms: 0,
            last_pool_ms: None,
            last_tick_sec: 0,
            data_version: 0,
            stats_cache: RefCell::new(None),
        };
        app.viz.set_motion(ui.motion);
        if ui.shape == "hexagon" {
            app.viz.shape = Shape::Hexagon;
        }
        app
    }

    /// The UI choices worth remembering across runs.
    pub fn ui_state(&self) -> UiState {
        UiState {
            theme: Some(self.theme.name.to_string()),
            view: Some(self.view.name().to_string()),
            sort: Some(self.sort.label().to_string()),
            sort_reversed: Some(self.sort_reversed),
            motion: Some(self.viz.motion),
            shape: Some(
                match self.viz.shape {
                    Shape::Rect => "rect",
                    Shape::Hexagon => "hexagon",
                }
                .to_string(),
            ),
        }
    }

    pub fn on_source_event(&mut self, ev: SourceEvent, now_ms: u64) {
        self.clock_ms = self.clock_ms.max(now_ms);
        if !matches!(ev, SourceEvent::Price(_) | SourceEvent::TokenMeta(_)) {
            self.data_version += 1;
        }
        match ev {
            SourceEvent::AddressBook(entries) => {
                self.book = entries;
                self.cls = Classifier::new(&self.builtin, &self.book, &self.local);
                self.rec.reclassify(&self.cls);
                self.relayout(false);
                self.clamp_selection();
            }
            SourceEvent::Price(p) => self.price = Some(p),
            SourceEvent::TokenMeta(m) => {
                self.tokens.insert(m.token_id.clone(), m);
            }
            other => {
                let mempool_from = match &other {
                    SourceEvent::Mempool { source, .. } => Some(source.clone()),
                    _ => None,
                };
                let updates = self.rec.apply(other, now_ms, &self.cls);
                if mempool_from.is_some() && mempool_from.as_ref() == self.rec.active() {
                    self.last_pool_ms = Some(now_ms);
                }
                self.on_updates(&updates, now_ms);
            }
        }
    }

    fn on_updates(&mut self, updates: &[Update], now_ms: u64) {
        let mut changed = false;
        let mut animate = true;
        for u in updates {
            match u {
                Update::Added(_) => changed = true,
                Update::Dropped(ids) => {
                    for id in ids {
                        self.leaving.remove(id);
                    }
                    self.viz.on_dropped(ids, now_ms);
                    changed = true;
                }
                Update::Mined { tx_ids, .. } => {
                    for id in tx_ids {
                        self.leaving.remove(id);
                    }
                    self.viz.on_mined(tx_ids, now_ms);
                    changed = true;
                }
                Update::Resynced => {
                    self.leaving.clear();
                    changed = true;
                    animate = false;
                }
                Update::BlockAdded(h) => {
                    self.banner = self.block_banner(*h);
                    self.block_flash_until = now_ms + BLOCK_FLASH_MS;
                }
                Update::SourcesChanged => {}
            }
        }
        if !changed {
            // A poll that only removes txs emits no update, but their sprites must become pending.
            let pool = self.rec.pool();
            changed = self
                .viz
                .placed()
                .any(|s| s.state == crate::viz::State::Active && !pool.contains_key(&s.id));
        }
        if changed {
            self.relayout(animate);
        }
        self.clamp_selection();
    }

    fn block_banner(&self, height: u32) -> Option<String> {
        let b = self
            .rec
            .recent_blocks()
            .into_iter()
            .find(|b| b.height == height)?;
        Some(format!(
            "⛏ Block #{} by {} · {} txs · {} ERG",
            format::thousands(height as u64),
            self.miner_name(b),
            b.tx_ids.len(),
            format::erg(b.miner_reward)
        ))
    }

    pub fn miner_name(&self, b: &Block) -> String {
        b.miner_address
            .as_deref()
            .and_then(|a| self.cls.lookup(a))
            .map(|c| c.name)
            .unwrap_or_else(|| "Other".into())
    }

    pub fn relayout(&mut self, animate: bool) {
        let pool = self.rec.pool();
        // Pending tracking follows motion, not this relayout's animation: a resize or
        // address-book reload must not lose txs that are waiting for their block.
        if self.viz.motion {
            for s in self.viz.placed() {
                if !pool.contains_key(&s.id) && !self.leaving.contains_key(&s.id) {
                    self.leaving.insert(
                        s.id.clone(),
                        VizItem {
                            id: s.id.clone(),
                            size_bytes: s.size_bytes,
                            fee: s.fee,
                            color: s.color,
                            pending: true,
                        },
                    );
                }
            }
        } else {
            self.leaving.clear();
        }
        self.leaving.retain(|id, _| !pool.contains_key(id));
        let mut items: Vec<VizItem> = pool
            .values()
            .map(|e| VizItem {
                id: e.tx.id.clone(),
                size_bytes: e.tx.size,
                fee: e.metrics.fee,
                color: rgb(e.class.class.color),
                pending: false,
            })
            .collect();
        items.extend(self.leaving.values().cloned());
        let capacity = self.max_block_size();
        self.viz.relayout(&items, capacity, self.clock_ms, !animate);
    }

    pub fn resize_viz(&mut self, w_cells: u16, h_cells: u16) {
        if self.viz.set_size(w_cells, h_cells * 2) {
            self.relayout(false);
        }
    }

    pub fn rows(&self) -> Vec<&TxEntry> {
        let mut v: Vec<&TxEntry> = self
            .rec
            .pool()
            .values()
            .filter(|e| matches_filter(e, &self.filter))
            .collect();
        match self.sort {
            SortKey::Rate => sort_keyed(&mut v, |e| {
                Reverse(e.metrics.fee as u128 * 1000 / e.tx.size.max(1) as u128)
            }),
            SortKey::Fee => sort_keyed(&mut v, |e| Reverse(e.metrics.fee)),
            SortKey::Value => sort_keyed(&mut v, |e| Reverse(e.metrics.value)),
            SortKey::Size => sort_keyed(&mut v, |e| Reverse(e.tx.size)),
            SortKey::Age => sort_keyed(&mut v, |e| e.first_seen_ms),
            SortKey::Origin => sort_keyed(&mut v, |e| e.class.class.name.as_str()),
        }
        if self.sort_reversed {
            v.reverse();
        }
        v
    }

    /// ▼ for largest/oldest first, ▲ for smallest/newest first (origin: ▲ is A→Z).
    pub fn sort_arrow(&self) -> &'static str {
        let natural_up = self.sort == SortKey::Origin;
        if natural_up != self.sort_reversed {
            "▲"
        } else {
            "▼"
        }
    }

    fn selected_source_url(&self) -> Option<String> {
        let views = self.rec.views();
        let v = views.get(self.source_sel)?;
        self.source_urls.get(&v.id).cloned()
    }

    pub fn is_whale(&self, e: &TxEntry) -> bool {
        self.whale_nano > 0 && e.metrics.value >= self.whale_nano
    }

    /// Fee-rate stats, recomputed only when source data or the block size changed.
    pub fn rate_stats(&self) -> Option<RateStats> {
        let key = (self.data_version, self.max_block_size());
        if let Some((k, stats)) = *self.stats_cache.borrow() {
            if k == key {
                return stats;
            }
        }
        let stats = rate_stats(
            self.rec.pool().values().map(|e| (e.metrics.fee, e.tx.size)),
            u64::from(key.1),
        );
        *self.stats_cache.borrow_mut() = Some((key, stats));
        stats
    }

    pub fn selected_entry(&self) -> Option<&TxEntry> {
        self.rows().get(self.selected).copied()
    }

    /// Keeps the selection on the same tx as rows shift; closes the detail popup if it left.
    fn clamp_selection(&mut self) {
        let (found, n, lost_id) = {
            let rows = self.rows();
            let found = self
                .selected_id
                .as_ref()
                .and_then(|id| rows.iter().position(|e| &e.tx.id == id));
            let lost_id = if found.is_none() {
                self.selected_id.clone()
            } else {
                None
            };
            (found, rows.len(), lost_id)
        };
        self.selected = match found {
            Some(p) => p,
            None if n == 0 => 0,
            None => self.selected.min(n - 1),
        };
        if let (Some(id), Overlay::Detail) = (&lost_id, self.overlay) {
            let msg = format!("tx {} left the mempool", format::short_id(id));
            self.overlay = Overlay::None;
            self.set_status(msg, self.clock_ms);
        }
        self.selected_id = self.rows().get(self.selected).map(|e| e.tx.id.clone());
        let s = self.rec.views().len();
        self.source_sel = if s == 0 {
            0
        } else {
            self.source_sel.min(s - 1)
        };
    }

    pub fn best_info(&self) -> Option<&NodeInfo> {
        let views = self.rec.views();
        self.rec
            .active()
            .and_then(|a| views.iter().find(|v| &v.id == a))
            .and_then(|v| v.info.as_ref())
            .or_else(|| views.iter().find_map(|v| v.info.as_ref()))
    }

    pub fn max_block_size(&self) -> u32 {
        self.best_info()
            .map(|i| i.max_block_size)
            .filter(|m| *m > 0)
            .unwrap_or(DEFAULT_MAX_BLOCK_SIZE)
    }

    pub fn utilization_pct(&self) -> u64 {
        let bytes: u64 = self.rec.pool().values().map(|e| e.tx.size as u64).sum();
        bytes * 100 / self.max_block_size().max(1) as u64
    }

    pub fn chain_height(&self) -> u32 {
        let info = self
            .rec
            .views()
            .iter()
            .filter_map(|v| v.info.as_ref())
            .map(|i| i.full_height)
            .max();
        let block = self.rec.recent_blocks().first().map(|b| b.height);
        info.into_iter().chain(block).max().unwrap_or(0)
    }

    pub fn active_label(&self) -> String {
        let views = self.rec.views();
        match self
            .rec
            .active()
            .and_then(|a| views.iter().find(|v| &v.id == a))
        {
            Some(v) if v.kind == SourceKind::Node => format!("{} (node)", v.id),
            Some(v) => format!("{} (explorer)", v.id),
            None => "none".into(),
        }
    }

    pub fn freshness(&self, now_ms: u64) -> Freshness {
        let views = self.rec.views();
        let active = self
            .rec
            .active()
            .and_then(|a| views.iter().find(|v| &v.id == a));
        match active {
            Some(v) => {
                let age = v.last_update_ms.map_or(0, |t| now_ms.saturating_sub(t));
                let limit = match v.kind {
                    SourceKind::Node => STALE_NODE_MS,
                    SourceKind::Explorer => STALE_EXPLORER_MS,
                };
                if age > limit {
                    Freshness::Stale(age)
                } else {
                    Freshness::Live(age)
                }
            }
            None if self.last_pool_ms.is_none()
                && views
                    .iter()
                    .any(|v| v.status == ergotop_core::model::SourceStatus::Unknown) =>
            {
                Freshness::Connecting
            }
            None => Freshness::Offline(self.last_pool_ms.map(|t| now_ms.saturating_sub(t))),
        }
    }

    pub fn source_summary(&self, now_ms: u64) -> String {
        let views = self.rec.views();
        let active = self
            .rec
            .active()
            .and_then(|a| views.iter().find(|v| &v.id == a));
        let name = active.map(|v| match v.kind {
            SourceKind::Node => v.id.0.clone(),
            SourceKind::Explorer => format!("explorer fallback: {}", v.id),
        });
        let dot = if active.is_some_and(|v| v.kind == SourceKind::Node) {
            "●"
        } else {
            "○"
        };
        let lead = match (self.freshness(now_ms), name) {
            (Freshness::Live(age), Some(n)) => format!("{dot} {n} · {} ago", format::age(age)),
            (Freshness::Stale(age), Some(n)) => format!("◐ {n} · stale {}", format::age(age)),
            (Freshness::Connecting, _) => "… connecting to sources".to_string(),
            (Freshness::Offline(Some(age)), _) => format!(
                "✕ offline · data {} old (press 3 for details)",
                format::age(age)
            ),
            _ => "✕ no data source (press 3 for details)".to_string(),
        };
        let dots: Vec<String> = views
            .iter()
            .map(|v| {
                format!(
                    "{}{}",
                    format::trunc(&v.id.0, 14),
                    if v.status.usable() { "●" } else { "○" }
                )
            })
            .collect();
        format!(" {lead}  [{}]", dots.join(" "))
    }

    pub fn address_label(&self, address: &str) -> String {
        if address == FEE_ADDRESS {
            return "fee".into();
        }
        self.cls
            .lookup(address)
            .map(|c| c.name)
            .unwrap_or_else(|| format::short_addr(address))
    }

    pub fn token_name(&self, token_id: &str) -> String {
        self.tokens
            .get(token_id)
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| format::short_id(token_id))
    }

    pub fn token_amount(&self, t: &Token) -> String {
        match self
            .tokens
            .get(&t.token_id)
            .map(|m| m.decimals)
            .filter(|d| *d > 0)
        {
            Some(d) => format!("{:.*}", d as usize, t.amount as f64 / 10f64.powi(d as i32)),
            None => t.amount.to_string(),
        }
    }

    pub fn set_status(&mut self, msg: String, now_ms: u64) {
        self.status = Some((msg, now_ms + STATUS_MS));
    }

    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.clock_ms = self.clock_ms.max(now_ms);
        // Ages and the freshness line change every second even without events.
        let sec = now_ms / 1000;
        let new_second = sec != self.last_tick_sec;
        self.last_tick_sec = sec;
        let expired = self.viz.expire_pending(now_ms);
        if !expired.is_empty() {
            for id in &expired {
                self.leaving.remove(id);
            }
            self.relayout(true);
        }
        let animating = self.viz.tick(now_ms);
        let had_status = self.status.is_some();
        if self
            .status
            .as_ref()
            .is_some_and(|(_, until)| now_ms >= *until)
        {
            self.status = None;
        }
        new_second || animating || had_status || now_ms < self.block_flash_until + 100
    }

    pub fn on_key(&mut self, key: KeyEvent, now_ms: u64) -> Action {
        self.clock_ms = self.clock_ms.max(now_ms);
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.clamp_selection();
            return Action::None;
        }
        if self.overlay != Overlay::None {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.overlay = Overlay::None;
                    return Action::None;
                }
                KeyCode::Char('c') | KeyCode::Char('e') if self.overlay == Overlay::Detail => {}
                KeyCode::Up | KeyCode::Char('k') if self.overlay == Overlay::Detail => {
                    self.scroll_detail(-1);
                    return Action::None;
                }
                KeyCode::Down | KeyCode::Char('j') if self.overlay == Overlay::Detail => {
                    self.scroll_detail(1);
                    return Action::None;
                }
                KeyCode::PageUp if self.overlay == Overlay::Detail => {
                    self.scroll_detail(-(PAGE as i64));
                    return Action::None;
                }
                KeyCode::PageDown if self.overlay == Overlay::Detail => {
                    self.scroll_detail(PAGE as i64);
                    return Action::None;
                }
                KeyCode::Home | KeyCode::Char('g') if self.overlay == Overlay::Detail => {
                    self.detail_scroll = 0;
                    return Action::None;
                }
                KeyCode::End | KeyCode::Char('G') if self.overlay == Overlay::Detail => {
                    self.detail_scroll = u16::MAX;
                    return Action::None;
                }
                _ => return Action::None,
            }
        }
        let action = match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('1') => self.set_view(View::Dashboard),
            KeyCode::Char('2') => self.set_view(View::Packing),
            KeyCode::Char('3') => self.set_view(View::Sources),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-(PAGE as i64)),
            KeyCode::PageDown => self.move_selection(PAGE as i64),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(i64::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(i64::MAX / 2),
            KeyCode::Enter => {
                if self.view == View::Sources {
                    self.show_only = !self.show_only;
                } else if self.selected_entry().is_some() {
                    self.overlay = Overlay::Detail;
                    self.detail_scroll = 0;
                }
                Action::None
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.sort_reversed = false;
                Action::None
            }
            KeyCode::Char('S') => {
                self.sort_reversed = !self.sort_reversed;
                Action::None
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                Action::None
            }
            KeyCode::Esc => {
                self.filter.clear();
                Action::None
            }
            KeyCode::Char('c') if self.view == View::Sources => match self.selected_source_url() {
                Some(url) => {
                    self.set_status(format!("Copied {url}"), now_ms);
                    Action::Copy(url)
                }
                None => Action::None,
            },
            KeyCode::Char('e') if self.view == View::Sources => self
                .selected_source_url()
                .map_or(Action::None, Action::Open),
            KeyCode::Char('c') => match self.selected_entry().map(|e| e.tx.id.clone()) {
                Some(id) => {
                    self.set_status(format!("Copied {}", format::short_id(&id)), now_ms);
                    Action::Copy(id)
                }
                None => Action::None,
            },
            KeyCode::Char('e') => match self.selected_entry().map(|e| e.tx.id.clone()) {
                Some(id) => Action::Open(format!("{EXPLORER_TX_URL}{id}")),
                None => Action::None,
            },
            KeyCode::Char('l') => {
                self.viz.shape = match self.viz.shape {
                    Shape::Rect => Shape::Hexagon,
                    Shape::Hexagon => Shape::Rect,
                };
                self.relayout(false);
                Action::None
            }
            KeyCode::Char('t') => {
                self.theme = self.theme.next();
                self.set_status(format!("Theme: {}", self.theme.name), now_ms);
                Action::None
            }
            KeyCode::Char('m') => {
                let on = !self.viz.motion;
                self.viz.set_motion(on);
                if !on {
                    // Motion off: exits are immediate, so pending txs are cleared too.
                    self.leaving.clear();
                    self.relayout(false);
                }
                self.set_status(format!("Motion {}", if on { "on" } else { "off" }), now_ms);
                Action::None
            }
            KeyCode::Char('r') => {
                self.set_status("Refreshing sources…".into(), now_ms);
                Action::Refresh
            }
            KeyCode::Char('?') => {
                self.overlay = Overlay::Help;
                Action::None
            }
            _ => Action::None,
        };
        self.clamp_selection();
        action
    }

    fn scroll_detail(&mut self, delta: i64) {
        let cur = i64::from(self.detail_scroll.min(u16::MAX - 1));
        self.detail_scroll = (cur + delta).clamp(0, i64::from(u16::MAX - 1)) as u16;
    }

    fn set_view(&mut self, view: View) -> Action {
        self.view = view;
        Action::None
    }

    fn move_selection(&mut self, delta: i64) -> Action {
        if self.view == View::Sources {
            let n = self.rec.views().len() as i64;
            self.source_sel = (self.source_sel as i64 + delta).clamp(0, (n - 1).max(0)) as usize;
        } else {
            let n = self.rows().len() as i64;
            self.selected = (self.selected as i64 + delta).clamp(0, (n - 1).max(0)) as usize;
            self.selected_id = self.rows().get(self.selected).map(|e| e.tx.id.clone());
        }
        Action::None
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use ergotop_core::classify::{BookEntry, Kind};
    use ergotop_core::config::{SourceSpec, UiConfig};
    use ergotop_core::ergotree::tree_to_address;
    use ergotop_core::model::{BoxData, Input, SourceId, SourceKind, Tx};

    pub const NOW: u64 = 1_790_978_100_000;
    pub const KUCOIN: &str = "9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1";
    pub const WALLET: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    pub const CONTRACT: &str = "4MQyMKvMbnCJG3aJ";
    pub const POOL_2MINERS: &str =
        "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY";

    pub fn tid(prefix: &str) -> String {
        format!("{prefix}{}", "0".repeat(64 - prefix.len()))
    }

    pub fn bx(address: &str, value: u64) -> BoxData {
        BoxData {
            box_id: format!("box-{address}-{value}"),
            value,
            address: address.into(),
            tokens: vec![],
        }
    }

    pub fn tx(id: &str, size: u32, from: &str, outputs: Vec<BoxData>) -> Tx {
        let total: u64 = outputs.iter().map(|o| o.value).sum::<u64>() + 1_000;
        let input = bx(from, total);
        Tx {
            id: tid(id),
            size,
            inputs: vec![Input {
                box_id: input.box_id.clone(),
                resolved: Some(input),
            }],
            outputs,
            creation_ts_ms: None,
        }
    }

    pub fn specs() -> Vec<SourceSpec> {
        vec![
            SourceSpec {
                id: SourceId("node-a".into()),
                kind: SourceKind::Node,
                url: "http://n:9053".into(),
            },
            SourceSpec {
                id: SourceId("p2p".into()),
                kind: SourceKind::Explorer,
                url: "https://p2p".into(),
            },
        ]
    }

    /// Four txs (fees: d4 > b2 > a1 > c3), a node and an explorer source, one block by 2Miners.
    pub fn sample_app() -> App {
        let fresh = tree_to_address(&format!("0008cd02{}", "33".repeat(32))).unwrap();
        let mut app = App::new(
            &specs(),
            ergotop_core::config::AddressesFile::default(),
            &UiConfig::default(),
        );
        app.resize_viz(60, 12);
        app.on_source_event(
            SourceEvent::AddressBook(vec![BookEntry {
                address: KUCOIN.into(),
                name: "Kucoin".into(),
                kind: Kind::Exchange,
            }]),
            NOW - 60_000,
        );
        app.on_source_event(
            SourceEvent::Info {
                source: SourceId("node-a".into()),
                info: NodeInfo {
                    full_height: 1_886_101,
                    headers_height: 1_886_101,
                    peers: 31,
                    app_version: "6.0.1".into(),
                    max_block_size: 1_271_009,
                    indexed_height: Some(1_886_101),
                },
            },
            NOW - 60_000,
        );
        let txs = vec![
            tx(
                "a1",
                412,
                KUCOIN,
                vec![bx(WALLET, 12_000_000_000), bx(FEE_ADDRESS, 1_500_000)],
            ),
            tx(
                "b2",
                2150,
                WALLET,
                vec![bx(CONTRACT, 5_000_000_000), bx(FEE_ADDRESS, 2_000_000)],
            ),
            tx(
                "c3",
                300,
                WALLET,
                vec![bx(&fresh, 1_000_000_000), bx(FEE_ADDRESS, 1_100_000)],
            ),
            tx(
                "d4",
                20_000,
                WALLET,
                vec![bx(CONTRACT, 100_000_000_000), bx(FEE_ADDRESS, 10_000_000)],
            ),
        ];
        let ids: Vec<String> = txs.iter().map(|t| t.id.clone()).collect();
        app.on_source_event(
            SourceEvent::Mempool {
                source: SourceId("node-a".into()),
                ids: ids.clone(),
                new_txs: txs,
                latency_ms: 23,
            },
            NOW - 30_000,
        );
        app.on_source_event(
            SourceEvent::Mempool {
                source: SourceId("p2p".into()),
                ids: ids[..2].to_vec(),
                new_txs: vec![],
                latency_ms: 900,
            },
            NOW - 20_000,
        );
        app.on_source_event(
            SourceEvent::Block {
                source: SourceId("node-a".into()),
                block: Block {
                    id: "hdr-1".into(),
                    height: 1_886_101,
                    timestamp_ms: NOW - 90_000,
                    size: 187_236,
                    tx_ids: vec!["cb".into()],
                    miner_address: Some(POOL_2MINERS.into()),
                    miner_reward: 12_000_000_000,
                },
            },
            NOW - 10_000,
        );
        // A fresh, unchanged poll so the sample reads as live at NOW.
        app.on_source_event(
            SourceEvent::Mempool {
                source: SourceId("node-a".into()),
                ids,
                new_txs: vec![],
                latency_ms: 21,
            },
            NOW - 1_000,
        );
        app.on_source_event(SourceEvent::Price(0.3262), NOW);
        app
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ergotop_core::model::SourceId;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ids(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|e| e.tx.id[..2].to_string())
            .collect()
    }

    #[test]
    fn rows_sort_by_each_key() {
        let mut app = sample_app();
        assert_eq!(app.sort, SortKey::Rate, "fee rate is the default sort");
        assert_eq!(ids(&app), vec!["c3", "a1", "b2", "d4"]);
        app.sort = SortKey::Fee;
        assert_eq!(ids(&app), vec!["d4", "b2", "a1", "c3"]);
        app.sort = SortKey::Value;
        assert_eq!(ids(&app), vec!["d4", "a1", "b2", "c3"]);
        app.sort = SortKey::Size;
        assert_eq!(ids(&app), vec!["d4", "b2", "a1", "c3"]);
        app.sort = SortKey::Origin;
        assert_eq!(app.rows()[0].class.class.name, "Contract");
    }

    #[test]
    fn shift_s_reverses_and_s_resets_direction() {
        let mut app = sample_app();
        assert_eq!(app.sort_arrow(), "▼");
        app.on_key(key(KeyCode::Char('S')), NOW);
        assert!(app.sort_reversed);
        assert_eq!(app.sort_arrow(), "▲");
        assert_eq!(ids(&app), vec!["d4", "b2", "a1", "c3"]);
        assert_eq!(
            app.selected_entry().unwrap().tx.id,
            tid("c3"),
            "selection follows its tx"
        );
        app.on_key(key(KeyCode::Char('s')), NOW);
        assert_eq!((app.sort, app.sort_reversed), (SortKey::Fee, false));
        app.sort = SortKey::Origin;
        assert_eq!(app.sort_arrow(), "▲", "origin sorts A→Z");
    }

    #[test]
    fn g_and_shift_g_jump_to_top_and_bottom() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        app.on_key(key(KeyCode::Char('G')), NOW);
        assert_eq!(app.selected, 3);
        app.on_key(key(KeyCode::Char('g')), NOW);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn detail_popup_scrolls_without_moving_the_selection() {
        let mut app = sample_app();
        app.on_key(key(KeyCode::Enter), NOW);
        app.on_key(key(KeyCode::Down), NOW);
        app.on_key(key(KeyCode::PageDown), NOW);
        assert_eq!(app.detail_scroll, 1 + PAGE as u16);
        assert_eq!(app.selected, 0);
        app.on_key(key(KeyCode::Home), NOW);
        assert_eq!(app.detail_scroll, 0);
        app.on_key(key(KeyCode::End), NOW);
        assert_eq!(app.detail_scroll, u16::MAX, "clamped to content when drawn");
        app.on_key(key(KeyCode::Esc), NOW);
        app.on_key(key(KeyCode::Enter), NOW);
        assert_eq!(app.detail_scroll, 0, "reopening starts at the top");
    }

    #[test]
    fn whales_are_txs_at_or_over_the_threshold() {
        let mut app = sample_app();
        let whales = |app: &App| {
            let mut v: Vec<String> = app
                .rows()
                .iter()
                .filter(|e| app.is_whale(e))
                .map(|e| e.tx.id[..2].to_string())
                .collect();
            v.sort();
            v
        };
        app.whale_nano = 12_000_000_000;
        assert_eq!(whales(&app), vec!["a1", "d4"]);
        app.whale_nano = 0;
        assert!(whales(&app).is_empty(), "0 disables whale highlighting");
    }

    #[test]
    fn filters_by_name_value_and_id() {
        let mut app = sample_app();
        app.filter = "kucoin".into();
        assert_eq!(ids(&app), vec!["a1"]);
        app.filter = ">50".into();
        assert_eq!(ids(&app), vec!["d4"]);
        app.filter = "<2".into();
        assert_eq!(ids(&app), vec!["c3"]);
        app.filter = "b2".into();
        assert_eq!(ids(&app), vec!["b2"]);
        app.filter = "exchange".into();
        assert_eq!(ids(&app), vec!["a1"]);
    }

    #[test]
    fn keys_switch_views_sort_and_overlays() {
        let mut app = sample_app();
        app.on_key(key(KeyCode::Char('1')), NOW);
        assert_eq!(app.view, View::Dashboard);
        app.on_key(key(KeyCode::Char('3')), NOW);
        assert_eq!(app.view, View::Sources);
        app.on_key(key(KeyCode::Char('2')), NOW);
        assert_eq!(app.view, View::Packing);
        app.on_key(key(KeyCode::Char('s')), NOW);
        assert_eq!(app.sort, SortKey::Fee);
        app.on_key(key(KeyCode::Char('?')), NOW);
        assert_eq!(app.overlay, Overlay::Help);
        app.on_key(key(KeyCode::Esc), NOW);
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(key(KeyCode::Char('l')), NOW);
        assert_eq!(app.viz.shape, ergotop_core::packing::Shape::Hexagon);
        let before = app.theme.name;
        app.on_key(key(KeyCode::Char('t')), NOW);
        assert_ne!(app.theme.name, before);
        assert_eq!(app.on_key(key(KeyCode::Char('q')), NOW), Action::Quit);
        assert_eq!(
            app.on_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                NOW
            ),
            Action::Quit
        );
    }

    #[test]
    fn filter_input_mode_edits_and_clears() {
        let mut app = sample_app();
        app.on_key(key(KeyCode::Char('/')), NOW);
        assert!(app.filtering);
        for c in "kuc".chars() {
            app.on_key(key(KeyCode::Char(c)), NOW);
        }
        app.on_key(key(KeyCode::Backspace), NOW);
        assert_eq!(app.filter, "ku");
        app.on_key(key(KeyCode::Enter), NOW);
        assert!(!app.filtering);
        assert_eq!(ids(&app), vec!["a1"]);
        app.on_key(key(KeyCode::Esc), NOW);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn selection_moves_clamps_and_survives_shrinking_rows() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        for _ in 0..10 {
            app.on_key(key(KeyCode::Down), NOW);
        }
        assert_eq!(app.selected, 3);
        app.on_key(key(KeyCode::Up), NOW);
        assert_eq!(app.selected, 2);
        app.selected = 3;
        app.filter = "kucoin".into();
        app.on_key(key(KeyCode::Char('x')), NOW);
        assert_eq!(app.selected, 0, "clamped after filter shrank rows to 1");
        assert!(app.selected_entry().is_some());
    }

    #[test]
    fn copy_and_open_act_on_selected_tx() {
        let mut app = sample_app();
        assert_eq!(
            app.on_key(key(KeyCode::Char('c')), NOW),
            Action::Copy(tid("c3"))
        );
        assert!(app.status.as_ref().unwrap().0.starts_with("Copied"));
        assert_eq!(
            app.on_key(key(KeyCode::Char('e')), NOW),
            Action::Open(format!(
                "https://explorer.ergoplatform.com/en/transactions/{}",
                tid("c3")
            ))
        );
        app.on_key(key(KeyCode::Enter), NOW);
        assert_eq!(app.overlay, Overlay::Detail);
        assert_eq!(
            app.on_key(key(KeyCode::Char('c')), NOW),
            Action::Copy(tid("c3"))
        );
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = sample_app();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('s'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        app.on_key(release, NOW);
        assert_eq!(app.sort, SortKey::Rate);
    }

    #[test]
    fn sources_view_selects_source_and_toggles_only_list() {
        let mut app = sample_app();
        app.view = View::Sources;
        app.on_key(key(KeyCode::Down), NOW);
        assert_eq!(app.source_sel, 1);
        app.on_key(key(KeyCode::Enter), NOW);
        assert!(app.show_only);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn copy_and_open_act_on_selected_source_in_sources_view() {
        let mut app = sample_app();
        app.view = View::Sources;
        app.on_key(key(KeyCode::Down), NOW);
        assert_eq!(
            app.on_key(key(KeyCode::Char('c')), NOW),
            Action::Copy("https://p2p".into())
        );
        assert_eq!(app.status.as_ref().unwrap().0, "Copied https://p2p");
        assert_eq!(
            app.on_key(key(KeyCode::Char('e')), NOW),
            Action::Open("https://p2p".into())
        );
    }

    #[test]
    fn mined_txs_leave_the_visualizer_and_blocks_raise_a_banner() {
        let mut app = sample_app();
        app.on_source_event(
            SourceEvent::Block {
                source: SourceId("node-a".into()),
                block: Block {
                    id: "hdr-2".into(),
                    height: 1_886_102,
                    timestamp_ms: NOW,
                    size: 1000,
                    tx_ids: vec!["cb2".into(), tid("a1")],
                    miner_address: Some(POOL_2MINERS.into()),
                    miner_reward: 12_000_000_000,
                },
            },
            NOW,
        );
        assert!(app.block_flash_until > NOW);
        assert!(app
            .banner
            .as_deref()
            .unwrap()
            .contains("Block #1,886,102 by 2Miners"));
        let remaining: Vec<String> = [tid("b2"), tid("c3"), tid("d4")].to_vec();
        app.on_source_event(
            SourceEvent::Mempool {
                source: SourceId("node-a".into()),
                ids: remaining,
                new_txs: vec![],
                latency_ms: 20,
            },
            NOW + 1_000,
        );
        assert!(app.viz.sprite(&tid("a1")).is_none());
        assert_eq!(app.viz.sprites().count(), 4, "3 placed + 1 flashing");
        assert!(app.tick(NOW + 1_100));
    }

    #[test]
    fn block_size_comes_from_node_info_else_default() {
        assert_eq!(sample_app().max_block_size(), 1_271_009);
        let app = App::new(&specs(), Default::default(), &Default::default());
        assert_eq!(app.max_block_size(), DEFAULT_MAX_BLOCK_SIZE);
        assert!(
            app.source_summary(NOW).contains("connecting"),
            "{}",
            app.source_summary(NOW)
        );
    }

    #[test]
    fn status_says_no_data_source_only_after_every_source_failed() {
        let mut app = App::new(&specs(), Default::default(), &Default::default());
        for id in ["node-a", "p2p"] {
            let down = ergotop_core::model::SourceStatus::Down("timeout".into());
            app.on_source_event(
                SourceEvent::Status {
                    source: SourceId(id.into()),
                    status: down,
                },
                NOW,
            );
        }
        assert!(
            app.source_summary(NOW).contains("no data source"),
            "{}",
            app.source_summary(NOW)
        );
    }

    #[test]
    fn freshness_goes_live_then_stale_then_offline() {
        let mut app = sample_app();
        assert_eq!(app.freshness(NOW), Freshness::Live(1_000));
        assert!(app.source_summary(NOW).contains("● node-a · 1s ago"));
        assert_eq!(app.freshness(NOW + 20_000), Freshness::Stale(21_000));
        assert!(app
            .source_summary(NOW + 20_000)
            .contains("◐ node-a · stale 21s"));
        for id in ["node-a", "p2p"] {
            app.on_source_event(
                SourceEvent::Status {
                    source: SourceId(id.into()),
                    status: ergotop_core::model::SourceStatus::Down("timeout".into()),
                },
                NOW + 30_000,
            );
        }
        assert_eq!(
            app.freshness(NOW + 30_000),
            Freshness::Offline(Some(31_000))
        );
        let s = app.source_summary(NOW + 30_000);
        assert!(s.contains("✕ offline · data 31s old"), "{s}");
    }

    #[test]
    fn rate_stats_follow_mempool_changes() {
        let mut app = sample_app();
        assert_eq!(app.rate_stats().unwrap().p90, 3_666);
        assert_eq!(app.rate_stats().unwrap().p90, 3_666, "cached read");
        app.on_source_event(
            node_mempool(vec![tid("a1"), tid("b2"), tid("d4")], vec![]),
            NOW + 1_000,
        );
        assert_eq!(app.rate_stats().unwrap().p90, 3_640, "c3 left: recomputed");
    }

    #[test]
    fn starts_from_configured_ui_and_reports_changes_to_remember() {
        let ui = UiConfig {
            theme: "blue-ice".into(),
            start_view: "dashboard".into(),
            sort: "value".into(),
            sort_reversed: true,
            shape: "hexagon".into(),
            motion: false,
            ..Default::default()
        };
        let mut app = App::new(&specs(), Default::default(), &ui);
        assert_eq!((app.sort, app.sort_reversed), (SortKey::Value, true));
        assert_eq!(app.viz.shape, Shape::Hexagon);
        let state = app.ui_state();
        let mut round = UiConfig::default();
        state.apply(&mut round);
        assert_eq!(
            round,
            UiConfig {
                fps: round.fps,
                whale_erg: round.whale_erg,
                ..ui
            }
        );
        app.on_key(key(KeyCode::Char('s')), NOW);
        assert_eq!(app.ui_state().sort.as_deref(), Some("size"));
        assert_eq!(app.ui_state().sort_reversed, Some(false));
    }

    #[test]
    fn r_asks_sources_to_refresh() {
        let mut app = sample_app();
        assert_eq!(app.on_key(key(KeyCode::Char('r')), NOW), Action::Refresh);
        assert_eq!(app.status.as_ref().unwrap().0, "Refreshing sources…");
    }

    #[test]
    fn labels_addresses_and_tokens() {
        let mut app = sample_app();
        assert_eq!(app.address_label(KUCOIN), "Kucoin");
        assert_eq!(app.address_label(FEE_ADDRESS), "fee");
        assert_eq!(app.address_label(WALLET), "9guaDYhH…Ym3Rsq");
        app.on_source_event(
            SourceEvent::TokenMeta(TokenMeta {
                token_id: "tok".into(),
                name: Some("SigUSD".into()),
                decimals: 2,
            }),
            NOW,
        );
        assert_eq!(app.token_name("tok"), "SigUSD");
        assert_eq!(
            app.token_amount(&Token {
                token_id: "tok".into(),
                amount: 12345
            }),
            "123.45"
        );
        assert_eq!(
            app.token_amount(&Token {
                token_id: "other".into(),
                amount: 7
            }),
            "7"
        );
    }

    fn block_with(height: u32, txs: Vec<String>) -> SourceEvent {
        SourceEvent::Block {
            source: SourceId("node-a".into()),
            block: Block {
                id: format!("hdr-{height}"),
                height,
                timestamp_ms: NOW,
                size: 1000,
                tx_ids: txs,
                miner_address: Some(POOL_2MINERS.into()),
                miner_reward: 12_000_000_000,
            },
        }
    }

    fn node_mempool(ids: Vec<String>, new_txs: Vec<ergotop_core::model::Tx>) -> SourceEvent {
        SourceEvent::Mempool {
            source: SourceId("node-a".into()),
            ids,
            new_txs,
            latency_ms: 20,
        }
    }

    #[test]
    fn mined_txs_flash_when_the_mempool_drops_them_before_the_block_arrives() {
        let mut app = sample_app();
        let e5 = tx(
            "e5",
            500,
            WALLET,
            vec![bx(CONTRACT, 2_000_000_000), bx(FEE_ADDRESS, 1_200_000)],
        );
        app.on_source_event(
            node_mempool(vec![tid("b2"), tid("c3"), tid("d4"), tid("e5")], vec![e5]),
            NOW + 1_000,
        );
        let a1 = app.viz.sprite(&tid("a1")).expect("a1 waits in place");
        assert!(matches!(a1.state, crate::viz::State::Pending { .. }));
        app.on_source_event(
            block_with(1_886_102, vec!["cb2".into(), tid("a1")]),
            NOW + 3_000,
        );
        assert!(app.viz.sprite(&tid("a1")).is_none());
        let flashing = app
            .viz
            .sprites()
            .filter(|s| matches!(s.state, crate::viz::State::Flashing { .. }))
            .count();
        assert_eq!(flashing, 1, "a1 launches from where it was built");
    }

    #[test]
    fn detail_follows_the_selected_tx_when_rows_shift() {
        let mut app = sample_app();
        app.sort = SortKey::Fee;
        app.on_key(key(KeyCode::Down), NOW);
        assert_eq!(app.selected_entry().unwrap().tx.id, tid("b2"));
        app.on_key(key(KeyCode::Enter), NOW);
        let f9 = tx(
            "f9",
            300,
            WALLET,
            vec![bx(CONTRACT, 1_000_000_000), bx(FEE_ADDRESS, 50_000_000)],
        );
        let ids = vec![tid("a1"), tid("b2"), tid("c3"), tid("d4"), tid("f9")];
        app.on_source_event(node_mempool(ids, vec![f9]), NOW + 1_000);
        assert_eq!(
            app.selected_entry().unwrap().tx.id,
            tid("b2"),
            "selection follows the tx, not the row"
        );
        assert_eq!(app.overlay, Overlay::Detail);
        assert_eq!(
            app.on_key(key(KeyCode::Char('c')), NOW + 1_000),
            Action::Copy(tid("b2"))
        );
    }

    #[test]
    fn detail_closes_when_its_tx_leaves_the_mempool() {
        let mut app = sample_app();
        app.on_key(key(KeyCode::Enter), NOW);
        assert_eq!(app.overlay, Overlay::Detail);
        app.on_source_event(
            node_mempool(vec![tid("a1"), tid("b2"), tid("d4")], vec![]),
            NOW + 1_000,
        );
        assert_eq!(app.overlay, Overlay::None, "no invisible popup left behind");
        assert!(app.status.as_ref().unwrap().0.contains("left the mempool"));
        assert_eq!(
            app.on_key(key(KeyCode::Char('q')), NOW + 1_000),
            Action::Quit
        );
    }

    #[test]
    fn pending_txs_fade_when_the_reconciler_drops_them() {
        let mut app = sample_app();
        let rest = vec![tid("b2"), tid("c3"), tid("d4")];
        app.on_source_event(node_mempool(rest.clone(), vec![]), NOW + 1_000);
        let a1 = app.viz.sprite(&tid("a1")).expect("a1 still holds its slot");
        assert!(
            matches!(a1.state, crate::viz::State::Pending { .. }),
            "a removal with no other change still marks the tx pending"
        );
        app.on_source_event(node_mempool(rest, vec![]), NOW + 17_000);
        assert!(app.viz.sprite(&tid("a1")).is_none());
        assert!(app
            .viz
            .sprites()
            .any(|s| matches!(s.state, crate::viz::State::Fading { .. })));
    }

    #[test]
    fn pending_txs_expire_after_the_hold() {
        let mut app = sample_app();
        app.on_source_event(
            node_mempool(vec![tid("b2"), tid("c3"), tid("d4")], vec![]),
            NOW + 1_000,
        );
        app.tick(NOW + 1_000 + crate::viz::PENDING_HOLD_MS - 1);
        assert!(app.viz.sprite(&tid("a1")).is_some());
        app.tick(NOW + 1_000 + crate::viz::PENDING_HOLD_MS);
        assert!(app.viz.sprite(&tid("a1")).is_none());
    }

    #[test]
    fn m_toggles_motion_and_config_sets_the_default() {
        let mut app = sample_app();
        assert!(app.viz.motion);
        app.on_key(key(KeyCode::Char('m')), NOW);
        assert!(!app.viz.motion);
        assert_eq!(app.status.as_ref().unwrap().0, "Motion off");
        app.on_key(key(KeyCode::Char('m')), NOW);
        assert!(app.viz.motion);
        let ui = ergotop_core::config::UiConfig {
            motion: false,
            ..Default::default()
        };
        assert!(!App::new(&specs(), Default::default(), &ui).viz.motion);
    }

    #[test]
    fn switching_views_keeps_pending_txs_so_their_block_still_launches() {
        let mut app = sample_app();
        app.on_source_event(
            node_mempool(vec![tid("b2"), tid("c3"), tid("d4")], vec![]),
            NOW + 1_000,
        );
        app.resize_viz(70, 14);
        let a1 = app
            .viz
            .sprite(&tid("a1"))
            .expect("still pending after a resize");
        assert!(matches!(a1.state, crate::viz::State::Pending { .. }));
        app.on_source_event(
            block_with(1_886_102, vec!["cb2".into(), tid("a1")]),
            NOW + 3_000,
        );
        let flashing = app
            .viz
            .sprites()
            .filter(|s| matches!(s.state, crate::viz::State::Flashing { .. }))
            .count();
        assert_eq!(flashing, 1);
    }

    #[test]
    fn motion_off_clears_pending_and_stops_tracking_it() {
        let mut app = sample_app();
        app.on_source_event(
            node_mempool(vec![tid("b2"), tid("c3"), tid("d4")], vec![]),
            NOW + 1_000,
        );
        assert!(app.viz.sprite(&tid("a1")).is_some());
        app.on_key(key(KeyCode::Char('m')), NOW + 1_000);
        assert!(
            app.viz.sprite(&tid("a1")).is_none(),
            "pending cleared when motion goes off"
        );
        app.on_source_event(
            node_mempool(vec![tid("c3"), tid("d4")], vec![]),
            NOW + 2_000,
        );
        assert!(
            app.viz.sprite(&tid("b2")).is_none(),
            "no new pending while motion is off"
        );
    }
}
