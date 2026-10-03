//! Dashboard: mempool summary and blocks | visualizer and tx table | network, origins, selection.
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use super::{kv, origin_counts, origin_text, panel, util_color};
use crate::app::App;
use crate::format;
use crate::theme::rgb;

pub fn draw(f: &mut Frame, area: Rect, app: &mut App, now_ms: u64) {
    let [left, center, right] = Layout::horizontal([
        Constraint::Length(33),
        Constraint::Min(40),
        Constraint::Length(36),
    ])
    .areas(area);
    let [summary_area, blocks_area] =
        Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(left);
    let [viz_area, table_area] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Min(0)]).areas(center);
    let [net_area, origin_area, detail_area] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Min(0),
    ])
    .areas(right);
    summary(f, summary_area, app, now_ms);
    blocks(f, blocks_area, app, now_ms);
    super::packing::viz_panel(f, viz_area, app, now_ms);
    tx_table(f, table_area, app, now_ms);
    network(f, net_area, app);
    origins(f, origin_area, app);
    selected(f, detail_area, app, now_ms);
}

fn bar(pct: u64, width: usize) -> String {
    let filled = pct.min(100) as usize * width / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "-".into())
}

fn summary(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let pool: Vec<_> = app.rec.pool().values().collect();
    let n = pool.len() as u64;
    let bytes: u64 = pool.iter().map(|e| e.tx.size as u64).sum();
    let fees: u64 = pool.iter().map(|e| e.metrics.fee).sum();
    let pct = app.utilization_pct();
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
        kv("Total fees", format!("{} ERG", format::fee(fees))),
        kv(
            "Avg fee",
            dash((n > 0).then(|| format!("{} ERG", format::fee(fees / n)))),
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

fn tx_table(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let rows = app.rows();
    let title = format!(
        "TRANSACTIONS {} · sort:{}{}",
        rows.len(),
        app.sort.label(),
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
    let shown: Vec<Row> = rows
        .iter()
        .skip(start)
        .take(visible)
        .map(|e| {
            Row::new(vec![
                Cell::from(format::short_id(&e.tx.id)),
                Cell::from(Line::from(vec![
                    Span::styled("■ ", Style::new().fg(rgb(e.class.class.color))),
                    Span::raw(format::trunc(&e.class.class.name, 12)),
                ])),
                Cell::from(format::fee(e.metrics.fee)),
                Cell::from(format!(
                    "{}{}",
                    format::erg(e.metrics.value),
                    if e.metrics.approx { "~" } else { "" }
                )),
                Cell::from(format::bytes(e.tx.size as u64)),
                Cell::from(format::age(now_ms.saturating_sub(e.first_seen_ms))),
            ])
        })
        .collect();
    let header = Row::new(vec!["ID", "Origin", "Fee", "Value", "Size", "Age"])
        .style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(8),
        Constraint::Length(14),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Min(6),
    ];
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

fn selected(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let lines = match app.selected_entry() {
        None => vec![Line::from("No transaction selected")],
        Some(e) => vec![
            kv("ID", format::short_id(&e.tx.id)),
            kv("Origin", origin_text(e)),
            kv("Fee", format!("{} ERG", format::fee(e.metrics.fee))),
            kv(
                "Value",
                format!(
                    "{} ERG{}",
                    format::erg(e.metrics.value),
                    if e.metrics.approx { " ~" } else { "" }
                ),
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
        ],
    };
    f.render_widget(
        Paragraph::new(lines).block(panel("SELECTED".into(), &t)),
        area,
    );
}
