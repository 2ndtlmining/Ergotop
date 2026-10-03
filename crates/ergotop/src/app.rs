//! Pure application state: folds source events and key presses; no terminal I/O.
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ergotop_core::classify::{BookEntry, Builtin, Classifier};
use ergotop_core::config::{AddressesFile, LocalAddress, SourceSpec, UiConfig};
use ergotop_core::metrics::FEE_ADDRESS;
use ergotop_core::model::{nano_to_erg, Block, NodeInfo, SourceKind, Token, TokenMeta};
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
const EXPLORER_TX_URL: &str = "https://explorer.ergoplatform.com/en/transactions/";

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Fee,
    Value,
    Size,
    Age,
    Origin,
}

impl SortKey {
    pub fn next(self) -> SortKey {
        match self {
            SortKey::Fee => SortKey::Value,
            SortKey::Value => SortKey::Size,
            SortKey::Size => SortKey::Age,
            SortKey::Age => SortKey::Origin,
            SortKey::Origin => SortKey::Fee,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
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
}

pub struct App {
    pub rec: Reconciler,
    pub cls: Classifier,
    builtin: Builtin,
    book: Vec<BookEntry>,
    local: Vec<LocalAddress>,
    pub tokens: HashMap<String, TokenMeta>,
    pub price: Option<f64>,
    pub view: View,
    pub overlay: Overlay,
    pub filter: String,
    pub filtering: bool,
    pub sort: SortKey,
    pub selected: usize,
    pub source_sel: usize,
    pub show_only: bool,
    pub theme: Theme,
    pub viz: Visualizer,
    pub status: Option<(String, u64)>,
    pub block_flash_until: u64,
    pub banner: Option<String>,
    last_tick_ms: u64,
}

fn matches_filter(e: &TxEntry, filter: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if let Some(n) = f.strip_prefix('>').and_then(|s| s.trim().parse::<f64>().ok()) {
        return nano_to_erg(e.metrics.value) >= n;
    }
    if let Some(n) = f.strip_prefix('<').and_then(|s| s.trim().parse::<f64>().ok()) {
        return nano_to_erg(e.metrics.value) <= n;
    }
    let f = f.to_lowercase();
    e.class.class.name.to_lowercase().contains(&f)
        || e.class.class.kind.label().to_lowercase().contains(&f)
        || e.class.from.as_deref().is_some_and(|x| x.to_lowercase().contains(&f))
        || e.tx.id.starts_with(&f)
}

impl App {
    pub fn new(specs: &[SourceSpec], addrs: AddressesFile, ui: &UiConfig) -> App {
        let builtin = Builtin::load();
        let cls = Classifier::new(&builtin, &[], &addrs.address);
        App {
            rec: Reconciler::new(specs.iter().map(|s| (s.id.clone(), s.kind)).collect()),
            cls,
            builtin,
            book: Vec::new(),
            local: addrs.address,
            tokens: HashMap::new(),
            price: None,
            view: View::parse(&ui.start_view),
            overlay: Overlay::None,
            filter: String::new(),
            filtering: false,
            sort: SortKey::Fee,
            selected: 0,
            source_sel: 0,
            show_only: false,
            theme: Theme::by_name(&ui.theme),
            viz: Visualizer::new(),
            status: None,
            block_flash_until: 0,
            banner: None,
            last_tick_ms: 0,
        }
    }

    pub fn on_source_event(&mut self, ev: SourceEvent, now_ms: u64) {
        match ev {
            SourceEvent::AddressBook(entries) => {
                self.book = entries;
                self.cls = Classifier::new(&self.builtin, &self.book, &self.local);
                self.rec.reclassify(&self.cls);
                self.relayout(false);
            }
            SourceEvent::Price(p) => self.price = Some(p),
            SourceEvent::TokenMeta(m) => {
                self.tokens.insert(m.token_id.clone(), m);
            }
            other => {
                let updates = self.rec.apply(other, now_ms, &self.cls);
                self.on_updates(&updates, now_ms);
            }
        }
    }

    fn on_updates(&mut self, updates: &[Update], now_ms: u64) {
        let mut changed = false;
        let mut animate = true;
        for u in updates {
            match u {
                Update::Added(_) | Update::Dropped(_) => changed = true,
                Update::Mined { tx_ids, .. } => {
                    self.viz.on_mined(tx_ids, now_ms);
                    changed = true;
                }
                Update::Resynced => {
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
        if changed {
            self.relayout(animate);
        }
        self.clamp_selection();
    }

    fn block_banner(&self, height: u32) -> Option<String> {
        let b = self.rec.recent_blocks().into_iter().find(|b| b.height == height)?;
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
        let items: Vec<VizItem> = self
            .rec
            .pool()
            .values()
            .map(|e| VizItem {
                id: e.tx.id.clone(),
                size_bytes: e.tx.size,
                fee: e.metrics.fee,
                color: rgb(e.class.class.color),
            })
            .collect();
        let capacity = self.max_block_size();
        self.viz.relayout(&items, capacity, animate);
    }

    pub fn resize_viz(&mut self, w_cells: u16, h_cells: u16) {
        if self.viz.set_size(w_cells, h_cells * 2) {
            self.relayout(false);
        }
    }

    pub fn rows(&self) -> Vec<&TxEntry> {
        let mut v: Vec<&TxEntry> = self.rec.pool().values().filter(|e| matches_filter(e, &self.filter)).collect();
        match self.sort {
            SortKey::Fee => v.sort_by(|a, b| b.metrics.fee.cmp(&a.metrics.fee).then_with(|| a.tx.id.cmp(&b.tx.id))),
            SortKey::Value => {
                v.sort_by(|a, b| b.metrics.value.cmp(&a.metrics.value).then_with(|| a.tx.id.cmp(&b.tx.id)))
            }
            SortKey::Size => v.sort_by(|a, b| b.tx.size.cmp(&a.tx.size).then_with(|| a.tx.id.cmp(&b.tx.id))),
            SortKey::Age => v.sort_by(|a, b| a.first_seen_ms.cmp(&b.first_seen_ms).then_with(|| a.tx.id.cmp(&b.tx.id))),
            SortKey::Origin => v.sort_by(|a, b| {
                a.class.class.name.cmp(&b.class.class.name).then_with(|| a.tx.id.cmp(&b.tx.id))
            }),
        }
        v
    }

    pub fn selected_entry(&self) -> Option<&TxEntry> {
        self.rows().get(self.selected).copied()
    }

    fn clamp_selection(&mut self) {
        let n = self.rows().len();
        self.selected = if n == 0 { 0 } else { self.selected.min(n - 1) };
        let s = self.rec.views().len();
        self.source_sel = if s == 0 { 0 } else { self.source_sel.min(s - 1) };
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
        let info = self.rec.views().iter().filter_map(|v| v.info.as_ref()).map(|i| i.full_height).max();
        let block = self.rec.recent_blocks().first().map(|b| b.height);
        info.into_iter().chain(block).max().unwrap_or(0)
    }

    pub fn active_label(&self) -> String {
        let views = self.rec.views();
        match self.rec.active().and_then(|a| views.iter().find(|v| &v.id == a)) {
            Some(v) if v.kind == SourceKind::Node => format!("{} (node)", v.id),
            Some(v) => format!("{} (explorer)", v.id),
            None => "none".into(),
        }
    }

    pub fn source_summary(&self) -> String {
        let views = self.rec.views();
        let lead = match self.rec.active().and_then(|a| views.iter().find(|v| &v.id == a)) {
            Some(v) if v.kind == SourceKind::Node => format!("● {}", v.id),
            Some(v) => format!("○ explorer fallback: {}", v.id),
            None => "✕ no data source".to_string(),
        };
        let dots: Vec<String> = views
            .iter()
            .map(|v| format!("{}{}", format::trunc(&v.id.0, 14), if v.status.usable() { "●" } else { "○" }))
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
        match self.tokens.get(&t.token_id).map(|m| m.decimals).filter(|d| *d > 0) {
            Some(d) => format!("{:.*}", d as usize, t.amount as f64 / 10f64.powi(d as i32)),
            None => t.amount.to_string(),
        }
    }

    pub fn set_status(&mut self, msg: String, now_ms: u64) {
        self.status = Some((msg, now_ms + STATUS_MS));
    }

    pub fn tick(&mut self, now_ms: u64) -> bool {
        let dt = if self.last_tick_ms == 0 { 0 } else { now_ms.saturating_sub(self.last_tick_ms).min(100) };
        self.last_tick_ms = now_ms;
        let animating = self.viz.tick(dt, now_ms);
        let had_status = self.status.is_some();
        if self.status.as_ref().is_some_and(|(_, until)| now_ms >= *until) {
            self.status = None;
        }
        animating || had_status || now_ms < self.block_flash_until + 100
    }

    pub fn on_key(&mut self, key: KeyEvent, now_ms: u64) -> Action {
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
            self.selected = 0;
            return Action::None;
        }
        if self.overlay != Overlay::None {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.overlay = Overlay::None;
                    return Action::None;
                }
                KeyCode::Char('c') | KeyCode::Char('e') if self.overlay == Overlay::Detail => {}
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
            KeyCode::Home => self.move_selection(i64::MIN / 2),
            KeyCode::End => self.move_selection(i64::MAX / 2),
            KeyCode::Enter => {
                if self.view == View::Sources {
                    self.show_only = !self.show_only;
                } else if self.selected_entry().is_some() {
                    self.overlay = Overlay::Detail;
                }
                Action::None
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
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
            KeyCode::Char('?') => {
                self.overlay = Overlay::Help;
                Action::None
            }
            _ => Action::None,
        };
        self.clamp_selection();
        action
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
    pub const POOL_2MINERS: &str = "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY";

    pub fn tid(prefix: &str) -> String {
        format!("{prefix}{}", "0".repeat(64 - prefix.len()))
    }

    fn bx(address: &str, value: u64) -> BoxData {
        BoxData { box_id: format!("box-{address}-{value}"), value, address: address.into(), tokens: vec![] }
    }

    fn tx(id: &str, size: u32, from: &str, outputs: Vec<BoxData>) -> Tx {
        let total: u64 = outputs.iter().map(|o| o.value).sum::<u64>() + 1_000;
        let input = bx(from, total);
        Tx {
            id: tid(id),
            size,
            inputs: vec![Input { box_id: input.box_id.clone(), resolved: Some(input) }],
            outputs,
            creation_ts_ms: None,
        }
    }

    pub fn specs() -> Vec<SourceSpec> {
        vec![
            SourceSpec { id: SourceId("node-a".into()), kind: SourceKind::Node, url: "http://n:9053".into() },
            SourceSpec { id: SourceId("p2p".into()), kind: SourceKind::Explorer, url: "https://p2p".into() },
        ]
    }

    /// Four txs (fees: d4 > b2 > a1 > c3), a node and an explorer source, one block by 2Miners.
    pub fn sample_app() -> App {
        let fresh = tree_to_address(&format!("0008cd02{}", "33".repeat(32))).unwrap();
        let mut app = App::new(&specs(), ergotop_core::config::AddressesFile::default(), &UiConfig::default());
        app.resize_viz(60, 12);
        app.on_source_event(
            SourceEvent::AddressBook(vec![BookEntry { address: KUCOIN.into(), name: "Kucoin".into(), kind: Kind::Exchange }]),
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
            tx("a1", 412, KUCOIN, vec![bx(WALLET, 12_000_000_000), bx(FEE_ADDRESS, 1_500_000)]),
            tx("b2", 2150, WALLET, vec![bx(CONTRACT, 5_000_000_000), bx(FEE_ADDRESS, 2_000_000)]),
            tx("c3", 300, WALLET, vec![bx(&fresh, 1_000_000_000), bx(FEE_ADDRESS, 1_100_000)]),
            tx("d4", 20_000, WALLET, vec![bx(CONTRACT, 100_000_000_000), bx(FEE_ADDRESS, 10_000_000)]),
        ];
        let ids: Vec<String> = txs.iter().map(|t| t.id.clone()).collect();
        app.on_source_event(
            SourceEvent::Mempool { source: SourceId("node-a".into()), ids: ids.clone(), new_txs: txs, latency_ms: 23 },
            NOW - 30_000,
        );
        app.on_source_event(
            SourceEvent::Mempool { source: SourceId("p2p".into()), ids: ids[..2].to_vec(), new_txs: vec![], latency_ms: 900 },
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
        app.rows().iter().map(|e| e.tx.id[..2].to_string()).collect()
    }

    #[test]
    fn rows_sort_by_each_key() {
        let mut app = sample_app();
        assert_eq!(ids(&app), vec!["d4", "b2", "a1", "c3"]);
        app.sort = SortKey::Value;
        assert_eq!(ids(&app), vec!["d4", "a1", "b2", "c3"]);
        app.sort = SortKey::Size;
        assert_eq!(ids(&app), vec!["d4", "b2", "a1", "c3"]);
        app.sort = SortKey::Origin;
        assert_eq!(app.rows()[0].class.class.name, "Contract");
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
        assert_eq!(app.sort, SortKey::Value);
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
        assert_eq!(app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), NOW), Action::Quit);
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
        assert_eq!(app.on_key(key(KeyCode::Char('c')), NOW), Action::Copy(tid("d4")));
        assert!(app.status.as_ref().unwrap().0.starts_with("Copied"));
        assert_eq!(
            app.on_key(key(KeyCode::Char('e')), NOW),
            Action::Open(format!("https://explorer.ergoplatform.com/en/transactions/{}", tid("d4")))
        );
        app.on_key(key(KeyCode::Enter), NOW);
        assert_eq!(app.overlay, Overlay::Detail);
        assert_eq!(app.on_key(key(KeyCode::Char('c')), NOW), Action::Copy(tid("d4")));
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = sample_app();
        let release = KeyEvent::new_with_kind(KeyCode::Char('s'), KeyModifiers::NONE, KeyEventKind::Release);
        app.on_key(release, NOW);
        assert_eq!(app.sort, SortKey::Fee);
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
        assert!(app.banner.as_deref().unwrap().contains("Block #1,886,102 by 2Miners"));
        let remaining: Vec<String> = [tid("b2"), tid("c3"), tid("d4")].to_vec();
        app.on_source_event(
            SourceEvent::Mempool { source: SourceId("node-a".into()), ids: remaining, new_txs: vec![], latency_ms: 20 },
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
        assert!(app.source_summary().contains("no data source"));
    }

    #[test]
    fn labels_addresses_and_tokens() {
        let mut app = sample_app();
        assert_eq!(app.address_label(KUCOIN), "Kucoin");
        assert_eq!(app.address_label(FEE_ADDRESS), "fee");
        assert_eq!(app.address_label(WALLET), "9guaDYhH…Ym3Rsq");
        app.on_source_event(
            SourceEvent::TokenMeta(TokenMeta { token_id: "tok".into(), name: Some("SigUSD".into()), decimals: 2 }),
            NOW,
        );
        assert_eq!(app.token_name("tok"), "SigUSD");
        assert_eq!(app.token_amount(&Token { token_id: "tok".into(), amount: 12345 }), "123.45");
        assert_eq!(app.token_amount(&Token { token_id: "other".into(), amount: 7 }), "7");
    }
}
