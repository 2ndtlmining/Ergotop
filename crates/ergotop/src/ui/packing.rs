//! Packing view: the next-block visualizer with an origin legend.
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{origin_counts, panel};
use crate::app::App;
use crate::canvas::Canvas;
use crate::format;
use crate::theme::rgb;

pub fn draw(f: &mut Frame, area: Rect, app: &mut App, now_ms: u64) {
    viz_panel(f, area, app, now_ms);
}

pub fn viz_panel(f: &mut Frame, area: Rect, app: &mut App, now_ms: u64) {
    let t = app.theme;
    let max = app.max_block_size() as u64;
    let r = &app.viz.last;
    let pct = r.block_bytes * 100 / max.max(1);
    let mut title = format!(
        "NEXT BLOCK  {} tx · {} / {} ({pct}%)",
        r.block_count,
        format::bytes(r.block_bytes),
        format::bytes(max)
    );
    if app.viz.shape == ergotop_core::packing::Shape::Hexagon {
        title.push_str(" · hexagon");
    }
    if r.not_shown > 0 {
        title.push_str(&format!(" · {} not shown", r.not_shown));
    }
    let block = panel(title, &t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height < 2 {
        return;
    }
    let [canvas_area, legend_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    app.resize_viz(canvas_area.width, canvas_area.height);
    let mut canvas = Canvas::new(canvas_area.width, canvas_area.height * 2);
    app.viz.render(&mut canvas, now_ms, t.dim);
    canvas.render(canvas_area, f.buffer_mut());
    let mut spans = Vec::new();
    for (name, color, n) in origin_counts(app).into_iter().take(6) {
        spans.push(Span::styled("■ ", Style::new().fg(rgb(color))));
        spans.push(Span::raw(format!("{} {n}  ", format::trunc(&name, 14))));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().fg(t.dim)), legend_area);
}
