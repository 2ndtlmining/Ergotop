//! Help and transaction-detail popups.
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::{kv, origin_text, panel};
use crate::app::App;
use crate::format;

const KEYS: [(&str, &str); 13] = [
    ("1 2 3", "Dashboard / Packing / Sources"),
    ("↑ ↓ PgUp PgDn", "Move selection"),
    ("Enter", "Transaction detail (Sources: txs only in source)"),
    ("s", "Cycle sort: rate → fee → value → size → age → origin"),
    ("/", "Filter: name, kind, tx id, >ERG, <ERG"),
    ("Esc", "Clear filter / close"),
    ("c", "Copy tx id (Sources: source URL)"),
    ("e", "Open tx in explorer (Sources: source URL)"),
    ("l", "Toggle hexagon packing"),
    ("t", "Cycle theme"),
    ("m", "Toggle motion"),
    ("?", "Help"),
    ("q", "Quit"),
];

fn centered(area: Rect, w_pct: u16, h_pct: u16) -> Rect {
    let w = area.width * w_pct / 100;
    let h = area.height * h_pct / 100;
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

pub fn help(f: &mut Frame, area: Rect, app: &App) {
    let r = centered(area, 70, 70);
    let lines: Vec<Line> = KEYS
        .iter()
        .map(|(k, d)| Line::from(format!("{k:<14} {d}")))
        .collect();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(panel("KEYS".into(), &app.theme)),
        r,
    );
}

pub fn detail(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let Some(e) = app.selected_entry() else {
        return;
    };
    let t = app.theme;
    let heading = Style::new().fg(t.accent).add_modifier(Modifier::BOLD);
    let r = centered(area, 85, 85);
    let mut lines = vec![
        kv("ID", e.tx.id.clone()),
        kv(
            "Origin",
            format!("{} ({})", origin_text(e), e.class.class.kind.label()),
        ),
        kv("Fee", format!("{} ERG", format::fee(e.metrics.fee))),
        kv(
            "Fee rate",
            format!(
                "{} n/B",
                format::rate(ergotop_core::metrics::fee_rate(e.metrics.fee, e.tx.size))
            ),
        ),
        kv(
            "Value",
            format!(
                "{} ERG{}",
                format::erg(e.metrics.value),
                if e.metrics.approx {
                    " (approx: inputs unresolved)"
                } else {
                    ""
                }
            ),
        ),
        kv("Size", format::bytes(e.tx.size as u64)),
        kv(
            "Seen",
            format!(
                "{} ago by {}",
                format::age(now_ms.saturating_sub(e.first_seen_ms)),
                e.seen_by
                    .iter()
                    .map(|s| s.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Line::from(""),
        Line::from(Span::styled(
            format!("INPUTS ({})", e.tx.inputs.len()),
            heading,
        )),
    ];
    for i in &e.tx.inputs {
        lines.push(match &i.resolved {
            Some(b) => Line::from(format!(
                "  {:<24} {:>16} ERG",
                app.address_label(&b.address),
                format::fee(b.value)
            )),
            None => Line::from(format!(
                "  {:<24} (unresolved)",
                format::short_id(&i.box_id)
            )),
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("OUTPUTS ({})", e.tx.outputs.len()),
        heading,
    )));
    for o in &e.tx.outputs {
        lines.push(Line::from(format!(
            "  {:<24} {:>16} ERG",
            app.address_label(&o.address),
            format::fee(o.value)
        )));
        for tok in &o.tokens {
            lines.push(Line::from(format!(
                "      + {} {}",
                app.token_amount(tok),
                app.token_name(&tok.token_id)
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "c: copy id · e: open in explorer · Esc: close",
        Style::new().fg(t.dim),
    )));
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(panel("TRANSACTION".into(), &t)),
        r,
    );
}
