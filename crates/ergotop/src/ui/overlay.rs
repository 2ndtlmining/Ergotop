//! Help and transaction-detail popups (filled in Task 6).
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;

pub fn help(_f: &mut Frame, _area: Rect, _app: &App) {}

pub fn detail(_f: &mut Frame, _area: Rect, _app: &App, _now_ms: u64) {}
