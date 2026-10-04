//! Rendering: header, status bar, view dispatch and shared panel helpers.
mod dashboard;
mod overlay;
pub mod packing;
mod sources;

use std::collections::HashMap;

use ergotop_core::classify::Rgb;
use ergotop_core::reconcile::TxEntry;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Freshness, Overlay, View};
use crate::format;
use crate::theme::Theme;

const HINTS: &str = " 1 2 3 views  / filter  s sort  c copy  e explorer  ? help  q quit ";

pub fn draw(f: &mut Frame, app: &mut App, now_ms: u64) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(app.theme.bg)), area);
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, head, app, now_ms);
    match app.view {
        View::Dashboard => dashboard::draw(f, body, app, now_ms),
        View::Packing => packing::draw(f, body, app, now_ms),
        View::Sources => sources::draw(f, body, app),
    }
    status_bar(f, foot, app, now_ms);
    match app.overlay {
        Overlay::Help => overlay::help(f, area, app),
        Overlay::Detail => overlay::detail(f, area, app, now_ms),
        Overlay::None => {}
    }
}

pub(crate) fn panel(title: String, theme: &Theme) -> Block<'static> {
    Block::bordered()
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(theme.dim))
        .style(Style::new().bg(theme.panel_bg).fg(theme.primary))
}

pub(crate) fn kv(k: &str, v: String) -> Line<'static> {
    Line::from(format!("{k:<11} {v}"))
}

pub(crate) fn util_color(pct: u64, t: &Theme) -> Color {
    if pct >= 95 {
        t.error
    } else if pct >= 80 {
        t.warning
    } else {
        t.primary
    }
}

/// (name, color, count) per classification, most common first.
pub(crate) fn origin_counts(app: &App) -> Vec<(&str, Rgb, usize)> {
    let mut m: HashMap<&str, (Rgb, usize)> = HashMap::new();
    for e in app.rec.pool().values() {
        let c = &e.class.class;
        m.entry(c.name.as_str()).or_insert((c.color, 0)).1 += 1;
    }
    let mut v: Vec<(&str, Rgb, usize)> = m.into_iter().map(|(n, (c, k))| (n, c, k)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    v
}

/// Which fixed-width columns fit in `avail` cells (1-cell gaps), added in `priority`
/// order; returns a keep-mask in display order. A column is shown whole or not at all.
pub(crate) fn fit_columns(widths: &[u16], priority: &[usize], avail: u16) -> Vec<bool> {
    let mut keep = vec![false; widths.len()];
    let mut used = 0u16;
    for &i in priority {
        let need = widths[i] + u16::from(used > 0);
        if used + need <= avail {
            keep[i] = true;
            used += need;
        }
    }
    keep
}

/// The items whose `keep` flag is set, in order.
pub(crate) fn kept<T>(items: Vec<T>, keep: &[bool]) -> Vec<T> {
    items
        .into_iter()
        .zip(keep)
        .filter_map(|(x, &k)| k.then_some(x))
        .collect()
}

pub(crate) fn origin_text(e: &TxEntry) -> String {
    match &e.class.from {
        Some(from) => format!("{from} → {}", e.class.class.name),
        None => e.class.class.name.clone(),
    }
}

fn header(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    if now_ms < app.block_flash_until {
        if let Some(b) = &app.banner {
            let style = Style::new()
                .bg(t.accent)
                .fg(t.bg)
                .add_modifier(Modifier::BOLD);
            f.render_widget(Paragraph::new(format!(" {b} ")).style(style), area);
            return;
        }
    }
    let pool_bytes: u64 = app.rec.pool().values().map(|e| e.tx.size as u64).sum();
    let pct = app.utilization_pct();
    let price = app
        .price
        .map(|p| format!("${p:.4}"))
        .unwrap_or_else(|| "-".into());
    let spans = vec![
        Span::styled(
            " ERGOTOP ",
            Style::new()
                .fg(t.bg)
                .bg(t.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  Block #{}  ",
            format::thousands(app.chain_height() as u64)
        )),
        Span::raw(format!("ERG {price}  ")),
        Span::raw(format!(
            "{} tx · {}  ",
            app.rec.pool().len(),
            format::bytes(pool_bytes)
        )),
        Span::styled(
            format!("mempool {pct}% of block"),
            Style::new().fg(util_color(pct, &t)),
        ),
    ];
    // Keep whole segments only: a cut-off "mempool 1% o" reads worse than nothing.
    let mut used = 0usize;
    let line = Line::from(
        spans
            .into_iter()
            .take_while(|s| {
                used += s.content.chars().count();
                used <= area.width as usize
            })
            .collect::<Vec<_>>(),
    );
    f.render_widget(
        Paragraph::new(line).style(Style::new().bg(t.panel_bg).fg(t.primary)),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let mut fg = t.primary;
    let left = if app.filtering {
        let errors = app.filter_errors();
        if let Some(first) = errors.first() {
            fg = t.error;
            format!(" /{}▏  ✕ {first}", app.filter)
        } else {
            format!(" /{}▏", app.filter)
        }
    } else if let Some((msg, _)) = &app.status {
        format!(" {msg}")
    } else {
        fg = match app.freshness(now_ms) {
            Freshness::Stale(_) => t.warning,
            Freshness::Offline(_) => t.error,
            Freshness::Connecting | Freshness::Live(_) => t.primary,
        };
        app.source_summary(now_ms)
    };
    let hint_w = (HINTS.chars().count() as u16).min(area.width.saturating_sub(30));
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(hint_w)]).areas(area);
    // The per-source dots are a bonus; drop them rather than cut them mid-list.
    let left = match left.find("  [") {
        Some(i) if left.chars().count() > l.width as usize => left[..i].to_string(),
        _ => left,
    };
    f.render_widget(
        Paragraph::new(left).style(Style::new().bg(t.panel_bg).fg(fg)),
        l,
    );
    f.render_widget(
        Paragraph::new(HINTS).style(Style::new().bg(t.panel_bg).fg(t.dim)),
        r,
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::testkit::{sample_app, specs, tid, NOW};
    use crate::app::View;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Renders twice (the first pass sizes the visualizer) and returns the screen text.
    pub(crate) fn screen(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app, NOW)).unwrap();
        term.draw(|f| draw(f, app, NOW)).unwrap();
        term.backend()
            .buffer()
            .content()
            .chunks(w as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_contains(s: &str, wants: &[&str]) {
        for want in wants {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
    }

    #[test]
    fn dashboard_shows_fee_rates_usd_and_whales() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        let s = screen(&mut app, 140, 40);
        assert_contains(
            &s,
            &[
                "Rate",
                "3,666",
                "Rate p50/90 930 / 3,666 n/B",
                "To get in   any fee",
                "0.0146 ERG (<$0.01)",
                "1.00 ERG ($0.33)",
            ],
        );
        assert!(!s.contains("WHALE"));
        app.whale_nano = 50_000_000_000;
        app.sort = crate::app::SortKey::Value;
        let s = screen(&mut app, 140, 40);
        assert_contains(&s, &["WHALE ≥ 50 ERG", "100.00 ERG ($32.62)"]);
    }

    #[test]
    fn fit_columns_adds_by_priority_and_never_cuts() {
        // widths 8, 14, 7; priority: 0, 2, 1
        assert_eq!(
            fit_columns(&[8, 14, 7], &[0, 2, 1], 100),
            vec![true, true, true]
        );
        assert_eq!(
            fit_columns(&[8, 14, 7], &[0, 2, 1], 20),
            vec![true, false, true]
        );
        assert_eq!(
            fit_columns(&[8, 14, 7], &[0, 2, 1], 15),
            vec![true, false, false]
        );
    }

    #[test]
    fn dashboard_numbers_are_whole_or_absent_at_any_width() {
        for (w, h) in [(60, 20), (80, 24), (100, 30), (120, 40), (140, 40)] {
            let mut app = sample_app();
            app.view = View::Dashboard;
            let s = screen(&mut app, w, h);
            assert_contains(&s, &["TRANSACTIONS 4", "c3000000", "3,666", "100.00"]);
            // A value cut from the left would read "0.00" right after a border or space.
            assert!(
                !s.contains(" 0.00 ") && !s.contains("│0.00"),
                "{w}x{h} cut a value:
{s}"
            );
        }
    }

    #[test]
    fn dashboard_keeps_mempool_and_selected_on_medium_terminals() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        let s = screen(&mut app, 100, 30);
        assert_contains(&s, &["MEMPOOL", "To get in", "SELECTED", "Fee rate"]);
        let s = screen(&mut app, 70, 24);
        assert!(
            !s.contains("NETWORK"),
            "narrow shows only visualizer + table:
{s}"
        );
    }

    #[test]
    fn filter_shows_match_count_and_flags_bad_terms() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        app.filter = "contract".into();
        assert_contains(&screen(&mut app, 140, 40), &["TRANSACTIONS 2/4"]);
        app.filtering = true;
        app.filter = "contract colour>3".into();
        assert_contains(
            &screen(&mut app, 140, 40),
            &["✕ colour>3: unknown field colour"],
        );
    }

    #[test]
    fn help_scrolls_on_short_terminals() {
        let mut app = sample_app();
        app.overlay = crate::app::Overlay::Help;
        let s = screen(&mut app, 100, 22);
        assert_contains(&s, &["↑↓ scroll"]);
        assert!(!s.contains("Quit"), "last line is below the fold");
        app.overlay_scroll = u16::MAX;
        assert_contains(&screen(&mut app, 100, 22), &["Quit"]);
    }

    #[test]
    fn header_drops_whole_segments_when_narrow() {
        let mut app = sample_app();
        app.view = View::Packing;
        let s = screen(&mut app, 70, 20);
        let top = s.lines().next().unwrap();
        assert!(
            top.contains("ERG $0.3262") && !top.contains("mempool"),
            "{top}"
        );
    }

    #[test]
    fn sources_view_keeps_key_columns_whole_at_80_columns() {
        let mut app = sample_app();
        app.view = View::Sources;
        let s = screen(&mut app, 80, 24);
        assert_contains(&s, &["node-a", "up", "21ms", "1,886,101"]);
    }

    #[test]
    fn dashboard_shows_all_panels() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        let s = screen(&mut app, 140, 40);
        assert_contains(
            &s,
            &[
                "ERGOTOP",
                "Block #1,886,101",
                "ERG $0.3262",
                "MEMPOOL",
                "RECENT BLOCKS",
                "2Miners",
                "NEXT BLOCK",
                "TRANSACTIONS 4",
                "Kucoin",
                "NETWORK",
                "ORIGINS",
                "SELECTED",
                "● node-a",
                "1tx 1m 30s",
                "q quit",
            ],
        );
        insta::assert_snapshot!("dashboard", s);
    }

    #[test]
    fn packing_view_shows_next_block_and_legend() {
        let mut app = sample_app();
        app.view = View::Packing;
        let s = screen(&mut app, 100, 30);
        assert_contains(&s, &["NEXT BLOCK  4 tx", "Contract", "▀", "q quit"]);
        insta::assert_snapshot!("packing", s);
    }

    #[test]
    fn hexagon_mode_is_labelled() {
        let mut app = sample_app();
        app.view = View::Packing;
        app.viz.shape = ergotop_core::packing::Shape::Hexagon;
        app.relayout(false);
        assert_contains(&screen(&mut app, 100, 30), &["hexagon"]);
    }

    #[test]
    fn empty_app_renders_without_sources() {
        let mut app = App::new(&specs(), Default::default(), &Default::default());
        for view in [View::Dashboard, View::Packing] {
            app.view = view;
            assert_contains(&screen(&mut app, 120, 30), &["connecting"]);
        }
    }

    #[test]
    fn filter_input_shows_in_status_bar() {
        let mut app = sample_app();
        app.filtering = true;
        app.filter = "kuc".into();
        assert_contains(&screen(&mut app, 120, 30), &["/kuc"]);
    }

    #[test]
    fn sources_view_compares_sources() {
        let mut app = sample_app();
        app.view = View::Sources;
        let s = screen(&mut app, 130, 30);
        assert_contains(
            &s,
            &[
                "SOURCES",
                "* node-a",
                "p2p",
                "Only here",
                "explorer",
                "6.0.1",
            ],
        );
        insta::assert_snapshot!("sources", s);
    }

    #[test]
    fn sources_view_lists_txs_only_in_selected_source() {
        let mut app = sample_app();
        app.view = View::Sources;
        app.show_only = true;
        let s = screen(&mut app, 130, 30);
        assert_contains(&s, &["ONLY IN node-a (2)", "c3000000", "d4000000"]);
    }

    #[test]
    fn help_overlay_lists_keys() {
        let mut app = sample_app();
        app.overlay = crate::app::Overlay::Help;
        let s = screen(&mut app, 120, 34);
        assert_contains(
            &s,
            &[
                "KEYS",
                "Cycle sort",
                "Toggle hexagon",
                "Toggle motion",
                "Quit",
            ],
        );
        insta::assert_snapshot!("help", s);
    }

    #[test]
    fn table_header_and_title_show_sort_direction() {
        let mut app = sample_app();
        app.view = View::Dashboard;
        assert_contains(&screen(&mut app, 140, 40), &["sort:rate ▼", "Rate▼"]);
        app.sort_reversed = true;
        assert_contains(&screen(&mut app, 140, 40), &["sort:rate ▲", "Rate▲"]);
    }

    #[test]
    fn detail_overlay_scrolls_and_links_the_explorer() {
        let mut app = sample_app();
        app.overlay = crate::app::Overlay::Detail;
        app.filter = "kucoin".into();
        let s = screen(&mut app, 140, 34);
        let url = format!(
            "https://explorer.ergoplatform.com/en/transactions/{}",
            tid("a1")
        );
        assert_contains(&s, &[url.as_str()]);
        // A short terminal cannot show the whole tx: the title says it scrolls,
        // and End clamps to the last page so Up moves right away.
        app.overlay_scroll = u16::MAX;
        let s = screen(&mut app, 120, 16);
        assert_contains(&s, &["↑↓ scroll", "Esc: close"]);
        let bottom = app.overlay_scroll;
        assert!(bottom > 0 && bottom < 30, "clamped, got {bottom}");
    }

    #[test]
    fn detail_overlay_shows_inputs_outputs_and_names() {
        let mut app = sample_app();
        app.overlay = crate::app::Overlay::Detail;
        app.filter = "kucoin".into();
        let s = screen(&mut app, 120, 34);
        assert_contains(
            &s,
            &[
                "TRANSACTION",
                "a1000000",
                "Kucoin",
                "INPUTS (1)",
                "OUTPUTS (2)",
                "fee",
                "12.00",
                "c: copy id",
            ],
        );
        let fee_line = s
            .lines()
            .find(|l| l.contains("│  fee "))
            .expect("fee output line");
        assert!(
            fee_line.contains("0.0015 ERG"),
            "small outputs must not round to zero: {fee_line}"
        );
        insta::assert_snapshot!("detail", s);
    }

    #[test]
    fn tiny_terminal_never_panics() {
        for view in [View::Dashboard, View::Packing, View::Sources] {
            for overlay in [
                crate::app::Overlay::None,
                crate::app::Overlay::Help,
                crate::app::Overlay::Detail,
            ] {
                let mut app = sample_app();
                app.view = view;
                app.overlay = overlay;
                app.show_only = true;
                screen(&mut app, 30, 8);
                screen(&mut app, 1, 1);
            }
        }
    }

    #[test]
    fn sources_view_shows_the_full_status_reason() {
        use ergotop_core::model::{SourceId, SourceStatus};
        use ergotop_core::sources::SourceEvent;
        let mut app = sample_app();
        app.view = View::Sources;
        let status = SourceStatus::Degraded("partial: 10 of 19 txs served".into());
        app.on_source_event(
            SourceEvent::Status {
                source: SourceId("p2p".into()),
                status,
            },
            NOW,
        );
        assert_contains(
            &screen(&mut app, 140, 30),
            &["degraded: partial: 10 of 19 txs served"],
        );
    }
}
