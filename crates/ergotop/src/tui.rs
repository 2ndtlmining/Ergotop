//! Terminal event loop: source events, key presses and an fps tick; redraw only when dirty.
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::execute;
use ergotop_core::config::{cache_dir, AddressesFile, Config};
use ergotop_core::sources::runtime::{spawn_all, Timing};
use futures::StreamExt;

use crate::app::{Action, App};
use crate::clock::now_ms;

fn frame_interval(fps: u32) -> Duration {
    Duration::from_millis(1000 / u64::from(fps.clamp(1, 120)))
}

pub async fn run(cfg: Config, addrs: AddressesFile, warnings: Vec<String>) -> anyhow::Result<()> {
    let specs = cfg.sources();
    let (mut rx, refresh) = spawn_all(&specs, Timing::default(), cache_dir());
    let mut app = App::new(&specs, addrs, &cfg.ui);
    if let Some(w) = warnings.first() {
        app.set_status(format!("warning: {w}"), now_ms());
    }
    let mut terminal = ratatui::init();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(frame_interval(cfg.ui.fps));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;

    let result = loop {
        if dirty {
            let now = now_ms();
            if let Err(e) = terminal.draw(|f| crate::ui::draw(f, &mut app, now)) {
                break Err(e.into());
            }
            dirty = false;
        }
        tokio::select! {
            Some(ev) = rx.recv() => {
                app.on_source_event(ev, now_ms());
                dirty = true;
            }
            maybe = events.next() => match maybe {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    match app.on_key(key, now_ms()) {
                        Action::Quit => break Ok(()),
                        Action::Copy(text) => {
                            let _ = execute!(std::io::stdout(), CopyToClipboard::to_clipboard_from(text));
                        }
                        Action::Open(url) => {
                            if webbrowser::open(&url).is_err() {
                                app.set_status(format!("Open: {url}"), now_ms());
                            }
                        }
                        Action::Refresh => refresh.now(),
                        Action::None => {}
                    }
                    dirty = true;
                }
                Some(Ok(Event::Resize(..))) => dirty = true,
                Some(Ok(_)) => {}
                Some(Err(e)) => break Err(e.into()),
                None => break Ok(()),
            },
            _ = tick.tick() => {
                if app.tick(now_ms()) {
                    dirty = true;
                }
            }
        }
    };
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_interval_follows_fps_with_bounds() {
        assert_eq!(frame_interval(30), Duration::from_millis(33));
        assert_eq!(frame_interval(0), Duration::from_millis(1000));
        assert_eq!(frame_interval(1000), Duration::from_millis(8));
    }
}
