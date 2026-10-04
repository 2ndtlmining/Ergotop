//! Dashboard: mempool summary and blocks | visualizer and tx table | network, origins, selection.
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use super::{kv, origin_counts, origin_text, panel, util_color};
use crate::app::{App, Freshness, SortKey};
use crate::format;
use crate::theme::rgb;
use ergotop_core::metrics::fee_rate;
use ergotop_core::model::nano_to_erg;
use ergotop_core::reconcile::TxEntry;

/// Below these widths the dashboard drops to two columns, then to one.
const WIDE: u16 = 120;
const MEDIUM: u16 = 80;

pub fn draw(f: &mut Frame, area: Rect, app: &mut App, now_ms: u64) {
    let (left, center, right) = if area.width >= WIDE {
        let [l, c, r] = Layout::horizontal([
            Constraint::Length(33),
            Constraint::Min(40),
            Constraint::Length(36),
        ])
        .areas(area);
        (Some(l), c, Some(r))
    } else if area.width >= MEDIUM {
        let [c, r] = Layout::horizontal([Constraint::Min(40), Constraint::Length(33)]).areas(area);
        (None, c, Some(r))
    } else {
        (None, area, None)
    };
    let [viz_area, table_area] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Min(0)]).areas(center);
    super::packing::viz_panel(f, viz_area, app, now_ms);
    // Filter + sort the pool once per frame; the table and the SELECTED panel share it.
    let rows = app.rows();
    tx_table(f, table_area, app, &rows, now_ms);
    let selected_entry = rows.get(app.selected).copied();
    match (left, right) {
        (Some(left), Some(right)) => {
            let [summary_area, blocks_area] =
                Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(left);
            let [net_area, origin_area, detail_area] = Layout::vertical([
                Constraint::Length(9),
                Constraint::Length(10),
                Constraint::Min(0),
            ])
            .areas(right);
            summary(f, summary_area, app, now_ms);
            blocks(f, blocks_area, app, now_ms);
            network(f, net_area, app);
            origins(f, origin_area, app);
            selected(f, detail_area, app, selected_entry, now_ms);
        }
        (None, Some(right)) => {
            // Network details live in the Sources view; origins in the visualizer legend.
            let [summary_area, detail_area] =
                Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(right);
            summary(f, summary_area, app, now_ms);
            selected(f, detail_area, app, selected_entry, now_ms);
        }
        _ => {}
    }
}

fn bar(pct: u64, width: usize) -> String {
    let filled = pct.min(100) as usize * width / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "-".into())
}

/// " ($1.23)" when the ERG price is known, else nothing.
fn usd_suffix(app: &App, nano: u64) -> String {
    app.price
        .map(|p| format!(" ({})", format::usd(nano_to_erg(nano) * p)))
        .unwrap_or_default()
}

fn summary(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let pool: Vec<_> = app.rec.pool().values().collect();
    let n = pool.len() as u64;
    let bytes: u64 = pool.iter().map(|e| e.tx.size as u64).sum();
    let fees: u64 = pool.iter().map(|e| e.metrics.fee).sum();
    let pct = app.utilization_pct();
    let stats = app.rate_stats();
    let width = area.width.saturating_sub(9) as usize;
    let lines = vec![
        Line::from(vec![
            Span::styled(bar(pct, width), Style::new().fg(util_color(pct, &t))),
            Span::raw(format!(" {pct}%")),
        ]),
        Line::from(format!(
            "{} / {}",
            format::bytes(bytes),
            format::bytes(app.max_block_size() as u64)
        )),
        kv(
            "Total fees",
            format!("{} ERG{}", format::fee(fees), usd_suffix(app, fees)),
        ),
        kv(
            "Rate p50/90",
            dash(
                stats.map(|r| format!("{} / {} n/B", format::rate(r.median), format::rate(r.p90))),
            ),
        ),
        kv(
            "To get in",
            dash(stats.map(|r| match r.entry {
                Some(e) => format!("≥ {} n/B", format::rate(e)),
                None => "any fee".into(),
            })),
        ),
        kv("Avg size", dash((n > 0).then(|| format::bytes(bytes / n)))),
        kv(
            "Largest",
            dash(
                pool.iter()
                    .map(|e| e.tx.size as u64)
                    .max()
                    .map(format::bytes),
            ),
        ),
        kv(
            "Oldest",
            dash(
                pool.iter()
                    .map(|e| e.first_seen_ms)
                    .min()
                    .map(|s| format::age(now_ms.saturating_sub(s))),
            ),
        ),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel("MEMPOOL".into(), &t)),
        area,
    );
}

fn blocks(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let lines: Vec<Line> = app
        .rec
        .recent_blocks()
        .into_iter()
        .map(|b| {
            Line::from(format!(
                "#{} {:<8} {:>2}tx {}",
                format::thousands(b.height as u64),
                format::trunc(&app.miner_name(b), 8),
                b.tx_ids.len(),
                format::age(now_ms.saturating_sub(b.timestamp_ms))
            ))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel("RECENT BLOCKS".into(), &app.theme)),
        area,
    );
}

fn tx_table(f: &mut Frame, area: Rect, app: &App, rows: &[&TxEntry], now_ms: u64) {
    let t = app.theme;
    let title = format!(
        "TRANSACTIONS {} · sort:{} {}{}{}",
        rows.len(),
        app.sort.label(),
        app.sort_arrow(),
        match app.freshness(now_ms) {
            Freshness::Stale(age) => format!(" · stale {}", format::age(age)),
            Freshness::Offline(Some(age)) => format!(" · offline, {} old", format::age(age)),
            _ => String::new(),
        },
        if app.filter.is_empty() {
            String::new()
        } else {
            format!(" · /{}", app.filter)
        }
    );
    let block = panel(title, &t);
    if area.height < 4 {
        f.render_widget(block, area);
        return;
    }
    let visible = (area.height - 3) as usize;
    let start = app.selected.saturating_sub(visible - 1);
    let shown: Vec<Vec<Cell>> = rows
        .iter()
        .skip(start)
        .take(visible)
        .map(|e| {
            let value = Line::from(format!(
                "{}{}",
                format::erg(e.metrics.value),
                if e.metrics.approx { "~" } else { " " }
            ))
            .right_aligned();
            let value = if app.is_whale(e) {
                value.style(Style::new().fg(t.warning).add_modifier(Modifier::BOLD))
            } else {
                value
            };
            vec![
                Cell::from(format::short_id(&e.tx.id)),
                Cell::from(Line::from(vec![
                    Span::styled("■ ", Style::new().fg(rgb(e.class.class.color))),
                    Span::raw(format::trunc(&e.class.class.name, 12)),
                ])),
                right(format::rate(fee_rate(e.metrics.fee, e.tx.size))),
                right(format::fee(e.metrics.fee)),
                Cell::from(value),
                right(format::bytes(e.tx.size as u64)),
                right(format::age(now_ms.saturating_sub(e.first_seen_ms))),
            ]
        })
        .collect();
    let label = |name: &str, key: SortKey| {
        if app.sort == key {
            format!("{name}{}", app.sort_arrow())
        } else {
            name.to_string()
        }
    };
    // ID, Origin, Rate, Fee, Value, Size, Age: shown whole by priority, never cut.
    const WIDTHS: [u16; 7] = [8, 14, 7, 7, 11, 8, 7];
    const PRIORITY: [usize; 7] = [0, 2, 4, 1, 6, 3, 5];
    let keep = super::fit_columns(&WIDTHS, &PRIORITY, area.width.saturating_sub(2));
    let shown: Vec<Row> = shown
        .into_iter()
        .map(|cells| Row::new(super::kept(cells, &keep)))
        .collect();
    let header = Row::new(super::kept(
        vec![
            Cell::from("ID"),
            Cell::from(label("Origin", SortKey::Origin)),
            right(label("Rate", SortKey::Rate)),
            right(label("Fee", SortKey::Fee)),
            right(format!("{:<6}", label("Value", SortKey::Value))),
            right(label("Size", SortKey::Size)),
            right(label("Age", SortKey::Age)),
        ],
        &keep,
    ))
    .style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD));
    let widths: Vec<Constraint> = super::kept(WIDTHS.map(Constraint::Length).to_vec(), &keep);
    let table = Table::new(shown, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::new().bg(t.cursor_bg).add_modifier(Modifier::BOLD));
    let mut state = TableState::default();
    if !rows.is_empty() {
        state.select(Some(app.selected - start));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn right(s: String) -> Cell<'static> {
    Cell::from(Line::from(s).right_aligned())
}

fn network(f: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![kv("Source", app.active_label())];
    match app.best_info() {
        Some(i) => {
            lines.push(kv("Height", format::thousands(i.full_height as u64)));
            lines.push(kv("Peers", i.peers.to_string()));
            lines.push(kv("Node", i.app_version.clone()));
            lines.push(kv(
                "Index lag",
                dash(
                    i.indexed_height
                        .map(|h| i.full_height.saturating_sub(h).to_string()),
                ),
            ));
        }
        None => lines.push(kv("Node", "no node data".into())),
    }
    lines.push(kv("Max block", format::bytes(app.max_block_size() as u64)));
    lines.push(kv("Mempool", format!("{} tx", app.rec.pool().len())));
    f.render_widget(
        Paragraph::new(lines).block(panel("NETWORK".into(), &app.theme)),
        area,
    );
}

fn origins(f: &mut Frame, area: Rect, app: &App) {
    let counts = origin_counts(app);
    let total = counts.iter().map(|c| c.2).sum::<usize>().max(1);
    let bar_w = area.width.saturating_sub(24) as usize;
    let lines: Vec<Line> = counts
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|(name, color, n)| {
            let pct = n * 100 / total;
            Line::from(vec![
                Span::styled("■ ", Style::new().fg(rgb(*color))),
                Span::raw(format!("{:<12} ", format::trunc(name, 12))),
                Span::styled("█".repeat(pct * bar_w / 100), Style::new().fg(rgb(*color))),
                Span::raw(format!(" {pct}%")),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel("ORIGINS".into(), &app.theme)),
        area,
    );
}

fn selected(f: &mut Frame, area: Rect, app: &App, entry: Option<&TxEntry>, now_ms: u64) {
    let t = app.theme;
    let lines = match entry {
        None => vec![Line::from("No transaction selected")],
        Some(e) => {
            let mut lines = vec![
                kv("ID", format::short_id(&e.tx.id)),
                kv("Origin", origin_text(e)),
                kv("Fee", format!("{} ERG", format::fee(e.metrics.fee))),
                kv(
                    "Value",
                    format!(
                        "{} ERG{}{}",
                        format::erg(e.metrics.value),
                        usd_suffix(app, e.metrics.value),
                        if e.metrics.approx { " ~" } else { "" },
                    ),
                ),
                kv(
                    "Fee rate",
                    format!("{} n/B", format::rate(fee_rate(e.metrics.fee, e.tx.size))),
                ),
                kv("Size", format::bytes(e.tx.size as u64)),
                kv(
                    "Seen",
                    format!(
                        "{} ago",
                        format::age(now_ms.saturating_sub(e.first_seen_ms))
                    ),
                ),
                kv(
                    "Sources",
                    e.seen_by
                        .iter()
                        .map(|s| s.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                kv(
                    "In / Out",
                    format!("{} / {}", e.tx.inputs.len(), e.tx.outputs.len()),
                ),
                Line::from(Span::styled("Enter: full detail", Style::new().fg(t.dim))),
            ];
            if app.is_whale(e) {
                lines.insert(
                    0,
                    Line::from(Span::styled(
                        format!("WHALE ≥ {} ERG", format::erg_whole(app.whale_nano)),
                        Style::new().fg(t.warning).add_modifier(Modifier::BOLD),
                    )),
                );
            }
            lines
        }
    };
    f.render_widget(
        Paragraph::new(lines).block(panel("SELECTED".into(), &t)),
        area,
    );
}
