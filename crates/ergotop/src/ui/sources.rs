//! Sources consistency view: one row per data source, plus txs only one source has.
use ergotop_core::model::{SourceKind, SourceStatus};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use super::panel;
use crate::app::App;
use crate::format;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let (table_area, only_area) = if app.show_only {
        let [a, b] = Layout::vertical([Constraint::Percentage(50), Constraint::Min(0)]).areas(area);
        (a, Some(b))
    } else {
        (area, None)
    };
    let views = app.rec.views();
    let rows: Vec<Row> = views
        .iter()
        .map(|v| {
            let active = app.rec.active() == Some(&v.id);
            let (status, color) = match &v.status {
                SourceStatus::Up => ("up".to_string(), t.primary),
                SourceStatus::Degraded(r) => (format!("degraded: {r}"), t.warning),
                SourceStatus::Down(r) => (format!("down: {r}"), t.error),
                SourceStatus::Unknown => ("waiting".to_string(), t.dim),
            };
            let info = v.info.as_ref();
            Row::new(vec![
                Cell::from(format!(
                    "{}{}",
                    if active { "* " } else { "  " },
                    format::trunc(&v.id.0, 14)
                )),
                Cell::from(match v.kind {
                    SourceKind::Node => "node",
                    SourceKind::Explorer => "explorer",
                }),
                Cell::from(Span::styled(
                    format::trunc(&status, 80),
                    Style::new().fg(color),
                )),
                Cell::from(
                    v.latency_ms
                        .map(|l| format!("{l}ms"))
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(v.ids.len().to_string()),
                Cell::from(app.rec.only_in(&v.id).len().to_string()),
                Cell::from(
                    info.map(|i| format::thousands(i.full_height as u64))
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(
                    info.and_then(|i| {
                        i.indexed_height
                            .map(|h| i.full_height.saturating_sub(h).to_string())
                    })
                    .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(
                    info.map(|i| i.app_version.clone())
                        .unwrap_or_else(|| "-".into()),
                ),
            ])
        })
        .collect();
    let header = Row::new(vec![
        "Source",
        "Kind",
        "Status",
        "Latency",
        "Txs",
        "Only here",
        "Height",
        "Index lag",
        "Version",
    ])
    .style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Min(30),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(9),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(panel(
            "SOURCES  (* active · ↑↓ select · Enter: txs only in source)".into(),
            &t,
        ))
        .row_highlight_style(Style::new().bg(t.cursor_bg).add_modifier(Modifier::BOLD));
    let mut state = TableState::default();
    if !views.is_empty() {
        state.select(Some(app.source_sel.min(views.len() - 1)));
    }
    f.render_stateful_widget(table, table_area, &mut state);

    if let (Some(only_area), Some(v)) = (only_area, views.get(app.source_sel)) {
        let ids = app.rec.only_in(&v.id);
        let lines: Vec<Line> = ids
            .iter()
            .map(|id| {
                let origin = app
                    .rec
                    .pool()
                    .get(id)
                    .map(|e| e.class.class.name.clone())
                    .unwrap_or_else(|| "-".into());
                Line::from(format!("{}  {origin}", format::short_id(id)))
            })
            .collect();
        let title = format!("ONLY IN {} ({})", v.id, ids.len());
        f.render_widget(Paragraph::new(lines).block(panel(title, &t)), only_area);
    }
}
