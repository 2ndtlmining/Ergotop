//! Sources consistency view (filled in Task 6).
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Paragraph::new("").block(super::panel("SOURCES".into(), &app.theme)), area);
}
