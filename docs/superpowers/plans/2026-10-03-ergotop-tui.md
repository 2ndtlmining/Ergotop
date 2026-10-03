# Ergotop TUI (Plan 2 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `ergotop` into the interactive terminal app from spec §4: Dashboard / Packing / Sources views, the animated gravity-packing visualizer, keys, themes, overlays, `--log`, and a frame-time benchmark.

**Architecture:** The `ergotop` crate becomes lib + bin. A pure `App` state machine (no terminal) folds `SourceEvent`s and key presses; `ui::draw` renders `&mut App` with ratatui; `tui::run` owns the terminal and a `tokio::select!` loop over source events, crossterm `EventStream`, and an fps tick, redrawing only when dirty. The visualizer packs with `ergotop_core::packing` and animates sprites on a half-block pixel `Canvas`.

**Tech Stack:** ratatui 0.30, crossterm 0.29 (`event-stream`, `osc52`), futures 0.3, webbrowser 1, tracing-subscriber 0.3; tests: insta 1, criterion 0.8 (bench). All APIs used here were compile-checked against these versions on 2026-10-03.

**Spec:** `docs/superpowers/specs/2026-10-03-ergotop-rust-design.md` (§4 TUI, §5 `[ui]` config, §6 errors, §7 snapshot tests + benchmark)

## Global Constraints

- Toolchain: Rust stable (Docker image `ergotop-rust` via `.superpowers/sdd/<plan>/cargo` wrapper on this machine; plain `cargo` elsewhere).
- No network access in tests; all UI tests use `ratatui::backend::TestBackend`.
- Classification colors (`ergotop_core::classify::Rgb`) render as `Color::Rgb`.
- Default 30 fps (`[ui] fps`, clamped 1..=120); redraw only when state is dirty or an animation/flash/status is active.
- Frame budget: rendering a 10,000-tx packing view must take < 5 ms (release build).
- `DEFAULT_MAX_BLOCK_SIZE = 1_271_009` bytes (explorer `/api/v1/epochs/params`, 2026-10-03) is used only when no node `/info` has reported `maxBlockSize`.
- Explorer link: `https://explorer.ergoplatform.com/en/transactions/{id}`.
- Copy uses OSC 52 (`crossterm::clipboard::CopyToClipboard`) so it works over SSH; opening a browser falls back to showing the URL in the status bar.
- Only `KeyEventKind::Press` is handled (Windows also reports releases).
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (`<trailer>` below).
- Work on branch `tui`.

## Review Focus

1. A very small terminal (e.g. 30×8) must render every view and overlay without panicking — pinned in Task 6.
2. Startup before any source has reported (empty pool, no active source) must render and say "no data source" — pinned in Task 5.
3. Key release/repeat events (Windows) must not double-trigger actions — pinned in Task 4.
4. The selection must stay valid when rows shrink (txs mined, filter applied) — pinned in Task 4.
5. A 10,000-tx mempool must keep frame rendering within budget — measured in Task 8.

---

## File Structure

```
Cargo.toml                                 + workspace deps (ratatui, crossterm, futures, webbrowser, tracing-subscriber, insta, criterion)
crates/ergotop/
  Cargo.toml                               lib + bin + bench
  src/lib.rs                               module list
  src/main.rs                              CLI: --config, --headless, --log
  src/format.rs                            erg/fee/bytes/age/thousands/short ids
  src/theme.rs                             4 themes + Rgb -> Color
  src/headless.rs                          (moved from bin into lib, unchanged)
  src/canvas.rs                            half-block pixel canvas
  src/viz.rs                               Visualizer: sprites, relayout, mined flash/leave, render
  src/app.rs                               App state machine + #[cfg(test)] testkit
  src/ui/mod.rs                            draw(): header, status bar, dispatch; shared helpers
  src/ui/dashboard.rs                      three-column dashboard
  src/ui/packing.rs                        packing panel (full view + compact)
  src/ui/sources.rs                        sources consistency view
  src/ui/overlay.rs                        help + tx detail popups
  src/ui/snapshots/*.snap                  insta snapshots
  src/tui.rs                               terminal loop
  benches/frame.rs                         criterion: relayout + render 10k txs
```

---

### Task 1: Library split, formatting helpers, themes

**Files:**
- Modify: `Cargo.toml`, `crates/ergotop/Cargo.toml`, `crates/ergotop/src/main.rs`
- Create: `crates/ergotop/src/lib.rs`, `crates/ergotop/src/format.rs`, `crates/ergotop/src/theme.rs`
- Move: `crates/ergotop/src/headless.rs` stays in place but becomes a lib module

**Interfaces:**
- Consumes: `ergotop_core::model::nano_to_erg`, `ergotop_core::classify::Rgb`.
- Produces:
  - `ergotop::format::{erg(u64)->String, fee(u64)->String, bytes(u64)->String, age(u64)->String, thousands(u64)->String, short_id(&str)->String, short_addr(&str)->String, trunc(&str, usize)->String}`
  - `ergotop::theme::{Theme, THEMES, rgb(Rgb)->Color}`; `Theme { name: &'static str, bg, panel_bg, primary, accent, warning, error, dim, cursor_bg: Color }` (Copy) with `Theme::by_name(&str) -> Theme`, `Theme::next(&self) -> Theme`
  - `ergotop::headless::run` (unchanged signature)

- [ ] **Step 1: Add dependencies**

Append to `[workspace.dependencies]` in `Cargo.toml`:

```toml
criterion = "0.8"
crossterm = { version = "0.29", features = ["event-stream", "osc52"] }
futures = "0.3"
insta = "1"
ratatui = "0.30"
tracing-subscriber = "0.3"
webbrowser = "1"
```

Replace `crates/ergotop/Cargo.toml` with:

```toml
[package]
name = "ergotop"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "ergotop"
path = "src/main.rs"

[dependencies]
ergotop-core = { path = "../ergotop-core" }
anyhow.workspace = true
clap.workspace = true
crossterm.workspace = true
futures.workspace = true
ratatui.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
webbrowser.workspace = true

[dev-dependencies]
criterion.workspace = true
insta.workspace = true

[[bench]]
name = "frame"
harness = false
```

Create `crates/ergotop/benches/frame.rs` with a placeholder-free minimal body so the manifest builds (Task 8 replaces it):

```rust
fn main() {}
```

- [ ] **Step 2: Create the lib and move headless into it**

`crates/ergotop/src/lib.rs`:

```rust
//! Ergotop terminal UI.
pub mod format;
pub mod headless;
pub mod theme;
```

In `crates/ergotop/src/main.rs` delete the line `mod headless;` and change the last line of `main` from `headless::run(cfg, addrs).await` to `ergotop::headless::run(cfg, addrs).await`.

Run: `cargo test -p ergotop`
Expected: the 2 existing headless tests PASS (now run as lib tests).

- [ ] **Step 3: Write failing tests for format and theme**

`crates/ergotop/src/format.rs`:

```rust
//! Display formatting shared by the TUI and headless output.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_amounts() {
        assert_eq!(erg(11_887_500_000), "11.89");
        assert_eq!(fee(1_500_000), "0.0015");
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(412), "412 B");
        assert_eq!(bytes(2150), "2.1 KB");
        assert_eq!(bytes(2_097_152), "2.00 MB");
    }

    #[test]
    fn formats_age() {
        assert_eq!(age(5_000), "5s");
        assert_eq!(age(252_000), "4m 12s");
        assert_eq!(age(7_380_000), "2h 03m");
    }

    #[test]
    fn formats_ids_and_numbers() {
        assert_eq!(thousands(1_886_101), "1,886,101");
        assert_eq!(thousands(999), "999");
        assert_eq!(short_id("abcdef0123456789"), "abcdef01");
        assert_eq!(short_addr("9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq"), "9guaDYhH…Ym3Rsq");
        assert_eq!(short_addr("4MQyMKvMbnCJG3aJ"), "4MQyMKvMbnCJG3aJ");
        assert_eq!(trunc("Rosen Bridge", 5), "Rosen");
    }
}
```

`crates/ergotop/src/theme.rs`:

```rust
//! Color themes ported from the Python version.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_and_cycles_themes() {
        assert_eq!(Theme::by_name("amber-terminal").name, "amber-terminal");
        assert_eq!(Theme::by_name("nope").name, "neon-green");
        let mut t = Theme::by_name("neon-green");
        let mut names = vec![];
        for _ in 0..4 {
            t = t.next();
            names.push(t.name);
        }
        assert_eq!(names, vec!["amber-terminal", "blue-ice", "high-contrast", "neon-green"]);
    }

    #[test]
    fn converts_classification_colors() {
        assert_eq!(rgb(ergotop_core::classify::Rgb(1, 2, 3)), Color::Rgb(1, 2, 3));
        assert_eq!(Theme::by_name("neon-green").primary, Color::Rgb(0x39, 0xff, 0x14));
    }
}
```

Update `lib.rs` (already lists `format` and `theme`).

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p ergotop`
Expected: FAIL to compile — `erg`, `bytes`, `Theme`, `rgb`, … not found.

- [ ] **Step 5: Implement**

Insert above the tests in `format.rs`:

```rust
use ergotop_core::model::nano_to_erg;

pub fn erg(nano: u64) -> String {
    format!("{:.2}", nano_to_erg(nano))
}

pub fn fee(nano: u64) -> String {
    format!("{:.4}", nano_to_erg(nano))
}

pub fn bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.2} MB", n as f64 / (1024.0 * 1024.0))
    }
}

pub fn age(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub fn short_addr(a: &str) -> String {
    let n = a.chars().count();
    if n <= 16 {
        return a.to_string();
    }
    let head: String = a.chars().take(8).collect();
    let tail: String = a.chars().skip(n - 6).collect();
    format!("{head}…{tail}")
}

pub fn trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
```

Insert above the tests in `theme.rs`:

```rust
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel_bg: Color,
    pub primary: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub dim: Color,
    pub cursor_bg: Color,
}

const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[allow(clippy::too_many_arguments)]
const fn theme(
    name: &'static str,
    bg: u32,
    panel_bg: u32,
    primary: u32,
    accent: u32,
    warning: u32,
    error: u32,
    dim: u32,
    cursor_bg: u32,
) -> Theme {
    Theme {
        name,
        bg: hex(bg),
        panel_bg: hex(panel_bg),
        primary: hex(primary),
        accent: hex(accent),
        warning: hex(warning),
        error: hex(error),
        dim: hex(dim),
        cursor_bg: hex(cursor_bg),
    }
}

pub const THEMES: [Theme; 4] = [
    theme("neon-green", 0x0a0e0f, 0x0c1213, 0x39ff14, 0x00ffcc, 0xffb000, 0xff4444, 0x4a7a4a, 0x1a3a1a),
    theme("amber-terminal", 0x0f0c06, 0x12100a, 0xffb000, 0xffd700, 0xff6600, 0xff4444, 0x7a6a3a, 0x3a2a0a),
    theme("blue-ice", 0x0a0e14, 0x0c1218, 0x4fc3f7, 0x80deea, 0xffb74d, 0xef5350, 0x37474f, 0x1a2a3a),
    theme("high-contrast", 0x000000, 0x0a0a0a, 0xffffff, 0x00ffff, 0xffff00, 0xff0000, 0x666666, 0x333333),
];

impl Theme {
    pub fn by_name(name: &str) -> Theme {
        THEMES.iter().copied().find(|t| t.name == name).unwrap_or(THEMES[0])
    }

    pub fn next(&self) -> Theme {
        let i = THEMES.iter().position(|t| t.name == self.name).unwrap_or(0);
        THEMES[(i + 1) % THEMES.len()]
    }
}

pub fn rgb(c: ergotop_core::classify::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ergotop`
Expected: 8 tests PASS (2 headless + 4 format + 2 theme).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ergotop
git commit -m "feat(tui): lib split, formatting helpers, themes

<trailer>"
```

---

### Task 2: Half-block canvas

**Files:**
- Create: `crates/ergotop/src/canvas.rs`
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod canvas;`)

**Interfaces:**
- Produces: `ergotop::canvas::Canvas` with `new(width: u16, height: u16) -> Canvas` (height in pixels, origin bottom-left), `width()`, `height()`, `get(x: u16, y: u16) -> Option<Color>`, `set(x: i32, y: i32, c: Color)` (clips), `fill(x: i32, y: i32, w: u16, h: u16, c: Color)` (clips), `render(&self, area: Rect, buf: &mut Buffer)` (two pixels per cell: top pixel → `▀` fg, bottom only → `▄` fg, both → `▀` fg/bg; empty cells untouched).

- [ ] **Step 1: Write the failing tests**

`crates/ergotop/src/canvas.rs`:

```rust
//! Half-block pixel canvas: each terminal cell shows two vertical pixels.

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgb(200, 0, 0);
    const BLUE: Color = Color::Rgb(0, 0, 200);

    fn rendered(c: &Canvas, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        c.render(area, &mut buf);
        buf
    }

    #[test]
    fn top_pixel_is_upper_half_block() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 1, RED);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!(b[(0, 0)].fg, RED);
    }

    #[test]
    fn bottom_pixel_is_lower_half_block() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 0, RED);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▄");
        assert_eq!(b[(0, 0)].fg, RED);
    }

    #[test]
    fn both_pixels_use_fg_and_bg() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 1, RED);
        c.set(0, 0, BLUE);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!((b[(0, 0)].fg, b[(0, 0)].bg), (RED, BLUE));
    }

    #[test]
    fn rows_map_bottom_up_and_empty_cells_are_untouched() {
        let mut c = Canvas::new(1, 4);
        c.set(0, 0, RED);
        let b = rendered(&c, 1, 2);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "▄");
    }

    #[test]
    fn writes_outside_the_canvas_are_clipped() {
        let mut c = Canvas::new(3, 3);
        c.set(-1, 0, RED);
        c.set(5, 5, RED);
        c.fill(2, 2, 4, 4, RED);
        assert_eq!(c.get(2, 2), Some(RED));
        assert_eq!(c.get(1, 1), None);
        assert_eq!(c.get(9, 9), None);
    }
}
```

Add `pub mod canvas;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop canvas`
Expected: FAIL to compile — `Canvas`, `Buffer`, `Rect`, `Color` not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;

pub struct Canvas {
    width: u16,
    height: u16,
    px: Vec<Option<Color>>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Self {
        Canvas { width, height, px: vec![None; width as usize * height as usize] }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }

    pub fn get(&self, x: u16, y: u16) -> Option<Color> {
        self.index(x as i32, y as i32).and_then(|i| self.px[i])
    }

    pub fn set(&mut self, x: i32, y: i32, c: Color) {
        if let Some(i) = self.index(x, y) {
            self.px[i] = Some(c);
        }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: u16, h: u16, c: Color) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.set(x + dx, y + dy, c);
            }
        }
    }

    fn at(&self, x: u16, y: i32) -> Option<Color> {
        self.index(x as i32, y).and_then(|i| self.px[i])
    }

    /// Draws into `area`; pixel rows map bottom-up, two per terminal row.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let rows = area.height.min(self.height.div_ceil(2));
        let cols = area.width.min(self.width);
        for cy in 0..rows {
            let top_y = self.height as i32 - 1 - 2 * cy as i32;
            for cx in 0..cols {
                let top = self.at(cx, top_y);
                let bottom = self.at(cx, top_y - 1);
                let Some(cell) = buf.cell_mut(Position::new(area.x + cx, area.y + cy)) else {
                    continue;
                };
                match (top, bottom) {
                    (None, None) => {}
                    (Some(t), None) => {
                        cell.set_char('▀').set_fg(t);
                    }
                    (None, Some(b)) => {
                        cell.set_char('▄').set_fg(b);
                    }
                    (Some(t), Some(b)) => {
                        cell.set_char('▀').set_fg(t).set_bg(b);
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop canvas`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop/src/canvas.rs crates/ergotop/src/lib.rs
git commit -m "feat(tui): half-block pixel canvas

<trailer>"
```

---

### Task 3: Animated packing visualizer

**Files:**
- Create: `crates/ergotop/src/viz.rs`
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod viz;`)

**Interfaces:**
- Consumes: `ergotop_core::packing::{pack, PackItem, PackParams, PackResult, Region, Shape}`, `crate::canvas::Canvas`.
- Produces (`ergotop::viz`):
  - consts `FALL_PX_PER_S: f32 = 60.0`, `LEAVE_PX_PER_S: f32 = 90.0`, `FLASH_MS: u64 = 600`, `MAX_SIDE: u16 = 6`
  - `pub enum Phase { Moving, Resting, Flashing { until_ms: u64 }, Leaving }` (Clone, Debug, PartialEq)
  - `pub struct Sprite { pub id: TxId, pub x: u16, pub y: f32, pub target_y: u16, pub side: u16, pub color: Color, pub region: Region, pub phase: Phase }`
  - `pub struct VizItem { pub id: TxId, pub size_bytes: u32, pub fee: u64, pub color: Color }`
  - `pub struct Visualizer { pub width: u16, pub height: u16, pub shape: Shape, pub last: PackResult, .. }` with `new()`, `params() -> PackParams` (block region = 3/4 of height), `set_size(w, h) -> bool`, `relayout(&mut self, items: &[VizItem], capacity: u32, animate: bool)`, `on_mined(&mut self, ids: &[TxId], now_ms: u64)`, `tick(&mut self, dt_ms: u64, now_ms: u64) -> bool` (true while animating), `sprite(&str) -> Option<&Sprite>`, `sprites() -> impl Iterator<Item = &Sprite>` (placed + leaving), `render(&self, canvas: &mut Canvas, now_ms: u64, line: Color)`

Behavior: new sprites start at the top (`y = height`) and fall to their packed `target_y` at `FALL_PX_PER_S`; `animate = false` (startup/resync/resize) places them directly. Mined sprites flash white/color for `FLASH_MS`, then rise at `LEAVE_PX_PER_S` and are removed once off the top. Sprites absent from a relayout (dropped / no longer shown) disappear. Overflow-region sprites render at half brightness. A dotted capacity line is drawn at `y = block_height` in `line` color.

- [ ] **Step 1: Write the failing tests**

`crates/ergotop/src/viz.rs`:

```rust
//! Animated packing visualizer: sprites fall into their packed positions.

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgb(200, 0, 0);

    fn item(id: &str, size: u32, fee: u64) -> VizItem {
        VizItem { id: id.into(), size_bytes: size, fee, color: RED }
    }

    #[test]
    fn relayout_without_animation_places_sprites_at_rest() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], 1_000_000, false);
        let s = v.sprite("a").unwrap();
        assert_eq!(s.phase, Phase::Resting);
        assert_eq!(s.target_y, 0);
        assert_eq!(s.y, 0.0);
    }

    #[test]
    fn new_sprites_fall_from_the_top_and_come_to_rest() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], 1_000_000, true);
        assert_eq!(v.sprite("a").unwrap().y, 20.0);
        assert!(v.tick(100, 0));
        assert!(v.sprite("a").unwrap().y < 20.0);
        for _ in 0..50 {
            v.tick(100, 0);
        }
        assert_eq!(v.sprite("a").unwrap().phase, Phase::Resting);
        assert!(!v.tick(100, 0), "nothing left to animate");
    }

    #[test]
    fn mined_sprites_flash_then_leave() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], 1_000_000, false);
        v.on_mined(&["a".to_string()], 1_000);
        assert!(v.sprite("a").is_none());
        assert_eq!(v.sprites().count(), 1);
        for i in 0..60 {
            v.tick(100, 1_000 + FLASH_MS + i * 100);
        }
        assert_eq!(v.sprites().count(), 0);
    }

    #[test]
    fn dropped_txs_disappear_on_relayout() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], 1_000_000, false);
        v.relayout(&[item("b", 1000, 10)], 1_000_000, false);
        assert!(v.sprite("a").is_none());
        assert!(v.sprite("b").is_some());
    }

    #[test]
    fn render_paints_block_sprites_and_dims_overflow() {
        let mut v = Visualizer::new();
        v.set_size(10, 8);
        v.relayout(&[item("a", 400, 4000), item("b", 400, 1)], 500, false);
        let a = v.sprite("a").unwrap().clone();
        let b = v.sprite("b").unwrap().clone();
        assert_eq!(a.region, Region::Block);
        assert_eq!(b.region, Region::Overflow);
        let mut c = Canvas::new(10, 8);
        v.render(&mut c, 0, Color::Gray);
        assert_eq!(c.get(a.x, a.target_y), Some(RED));
        assert_eq!(c.get(b.x, b.target_y), Some(Color::Rgb(100, 0, 0)));
    }
}
```

Add `pub mod viz;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop viz`
Expected: FAIL to compile — `Visualizer`, `VizItem`, `Phase`, … not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
use std::collections::HashMap;

use ergotop_core::model::TxId;
use ergotop_core::packing::{pack, PackItem, PackParams, PackResult, Region, Shape};
use ratatui::style::Color;

use crate::canvas::Canvas;

pub const FALL_PX_PER_S: f32 = 60.0;
pub const LEAVE_PX_PER_S: f32 = 90.0;
pub const FLASH_MS: u64 = 600;
pub const MAX_SIDE: u16 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Moving,
    Resting,
    Flashing { until_ms: u64 },
    Leaving,
}

#[derive(Clone, Debug)]
pub struct Sprite {
    pub id: TxId,
    pub x: u16,
    pub y: f32,
    pub target_y: u16,
    pub side: u16,
    pub color: Color,
    pub region: Region,
    pub phase: Phase,
}

pub struct VizItem {
    pub id: TxId,
    pub size_bytes: u32,
    pub fee: u64,
    pub color: Color,
}

pub struct Visualizer {
    pub width: u16,
    pub height: u16,
    pub shape: Shape,
    pub last: PackResult,
    sprites: HashMap<TxId, Sprite>,
    leaving: Vec<Sprite>,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

fn dim(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(r / 2, g / 2, b / 2),
        other => other,
    }
}

impl Visualizer {
    pub fn new() -> Self {
        Visualizer {
            width: 0,
            height: 0,
            shape: Shape::Rect,
            last: PackResult::default(),
            sprites: HashMap::new(),
            leaving: Vec::new(),
        }
    }

    pub fn params(&self, capacity: u32) -> PackParams {
        let block_height = self.height * 3 / 4;
        PackParams {
            width: self.width,
            block_height,
            overflow_height: self.height - block_height,
            capacity_bytes: capacity,
            max_side: MAX_SIDE,
            shape: self.shape,
        }
    }

    pub fn set_size(&mut self, width: u16, height: u16) -> bool {
        if (width, height) == (self.width, self.height) {
            return false;
        }
        self.width = width;
        self.height = height;
        true
    }

    pub fn sprite(&self, id: &str) -> Option<&Sprite> {
        self.sprites.get(id)
    }

    pub fn sprites(&self) -> impl Iterator<Item = &Sprite> {
        self.sprites.values().chain(self.leaving.iter())
    }

    pub fn relayout(&mut self, items: &[VizItem], capacity: u32, animate: bool) {
        let pack_items: Vec<PackItem> = items
            .iter()
            .map(|i| PackItem { id: i.id.clone(), size_bytes: i.size_bytes, fee: i.fee })
            .collect();
        let colors: HashMap<&str, Color> = items.iter().map(|i| (i.id.as_str(), i.color)).collect();
        let result = pack(&pack_items, &self.params(capacity));
        let mut next = HashMap::with_capacity(result.placed.len());
        for p in &result.placed {
            let color = colors.get(p.id.as_str()).copied().unwrap_or(Color::Gray);
            let sprite = match self.sprites.remove(&p.id) {
                Some(mut s) => {
                    s.x = p.x;
                    s.target_y = p.y;
                    s.side = p.side;
                    s.region = p.region;
                    s.color = color;
                    if !animate {
                        s.y = p.y as f32;
                    }
                    s.phase = if s.y == p.y as f32 { Phase::Resting } else { Phase::Moving };
                    s
                }
                None => Sprite {
                    id: p.id.clone(),
                    x: p.x,
                    y: if animate { self.height as f32 } else { p.y as f32 },
                    target_y: p.y,
                    side: p.side,
                    color,
                    region: p.region,
                    phase: if animate { Phase::Moving } else { Phase::Resting },
                },
            };
            next.insert(p.id.clone(), sprite);
        }
        self.sprites = next;
        self.last = result;
    }

    pub fn on_mined(&mut self, ids: &[TxId], now_ms: u64) {
        for id in ids {
            if let Some(mut s) = self.sprites.remove(id) {
                s.phase = Phase::Flashing { until_ms: now_ms + FLASH_MS };
                self.leaving.push(s);
            }
        }
    }

    pub fn tick(&mut self, dt_ms: u64, now_ms: u64) -> bool {
        let dt = dt_ms as f32 / 1000.0;
        let mut animating = false;
        for s in self.sprites.values_mut() {
            if s.phase != Phase::Moving {
                continue;
            }
            let target = s.target_y as f32;
            s.y = if s.y > target {
                (s.y - FALL_PX_PER_S * dt).max(target)
            } else {
                (s.y + FALL_PX_PER_S * dt).min(target)
            };
            if s.y == target {
                s.phase = Phase::Resting;
            } else {
                animating = true;
            }
        }
        for s in self.leaving.iter_mut() {
            match s.phase {
                Phase::Flashing { until_ms } if now_ms >= until_ms => s.phase = Phase::Leaving,
                Phase::Leaving => s.y += LEAVE_PX_PER_S * dt,
                _ => {}
            }
        }
        let top = self.height as f32;
        self.leaving.retain(|s| s.y < top);
        animating || !self.leaving.is_empty()
    }

    pub fn render(&self, canvas: &mut Canvas, now_ms: u64, line: Color) {
        let block_height = self.height * 3 / 4;
        if block_height < self.height {
            for x in (0..self.width).step_by(2) {
                canvas.set(x as i32, block_height as i32, line);
            }
        }
        for s in self.sprites() {
            let color = match s.phase {
                Phase::Flashing { .. } if (now_ms / 100) % 2 == 0 => Color::White,
                _ if s.region == Region::Overflow => dim(s.color),
                _ => s.color,
            };
            canvas.fill(s.x as i32, s.y.round() as i32, s.side, s.side, color);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop viz`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop/src/viz.rs crates/ergotop/src/lib.rs
git commit -m "feat(tui): animated packing visualizer

<trailer>"
```

---

### Task 4: App state machine

**Files:**
- Create: `crates/ergotop/src/app.rs`
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod app;`)

**Interfaces:**
- Consumes: `ergotop_core::{classify::{BookEntry, Builtin, Classifier}, config::{AddressesFile, LocalAddress, SourceSpec, UiConfig}, metrics::FEE_ADDRESS, model::*, packing::Shape, reconcile::{Reconciler, TxEntry, Update}, sources::SourceEvent}`, `crate::{format, theme::{rgb, Theme}, viz::{Visualizer, VizItem}}`, crossterm `KeyEvent`.
- Produces (`ergotop::app`):
  - `pub const DEFAULT_MAX_BLOCK_SIZE: u32 = 1_271_009;`
  - `pub enum View { Dashboard, Packing, Sources }` with `parse(&str)`; `pub enum SortKey { Fee, Value, Size, Age, Origin }` with `next()`, `label()`; `pub enum Overlay { None, Help, Detail }`; `pub enum Action { None, Quit, Copy(String), Open(String) }`
  - `pub struct App` with pub fields `rec, cls, tokens, price, view, overlay, filter, filtering, sort, selected, source_sel, show_only, theme, viz, status: Option<(String, u64)>, block_flash_until: u64, banner: Option<String>`
  - methods: `new(specs: &[SourceSpec], addrs: AddressesFile, ui: &UiConfig) -> App`, `on_source_event(SourceEvent, now_ms)`, `on_key(KeyEvent, now_ms) -> Action`, `tick(now_ms) -> bool`, `set_status(String, now_ms)`, `rows() -> Vec<&TxEntry>`, `selected_entry() -> Option<&TxEntry>`, `max_block_size() -> u32`, `utilization_pct() -> u64`, `chain_height() -> u32`, `best_info() -> Option<&NodeInfo>`, `active_label() -> String`, `source_summary() -> String`, `miner_name(&Block) -> String`, `address_label(&str) -> String`, `token_name(&str) -> String`, `token_amount(&Token) -> String`, `resize_viz(w_cells: u16, h_cells: u16)`, `relayout(animate: bool)`
  - `#[cfg(test)] pub(crate) mod testkit { NOW, tid(&str) -> String, sample_app() -> App }`

Filter syntax (spec §4.3): `>N` / `<N` compare value in ERG; anything else matches (case-insensitive) class name, kind label, input-side name, or tx-id prefix. Sorts: fee/value/size descending, age oldest first, origin by name; ties by id.

- [ ] **Step 1: Write the failing tests (and the test kit)**

`crates/ergotop/src/app.rs`:

```rust
//! Pure application state: folds source events and key presses; no terminal I/O.

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
```

Add `pub mod app;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop app`
Expected: FAIL to compile — `App`, `View`, `SortKey`, … not found.

- [ ] **Step 3: Implement**

Insert at the top of `app.rs`, above `testkit`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop app`
Expected: 11 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop/src/app.rs crates/ergotop/src/lib.rs
git commit -m "feat(tui): app state machine with keys, sort, filter, selection

<trailer>"
```

---

### Task 5: Header, status bar, dashboard and packing views

**Files:**
- Create: `crates/ergotop/src/ui/mod.rs`, `crates/ergotop/src/ui/dashboard.rs`, `crates/ergotop/src/ui/packing.rs`, `crates/ergotop/src/ui/sources.rs` (stub for this task), `crates/ergotop/src/ui/overlay.rs` (stub for this task)
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod ui;`)

**Interfaces:**
- Consumes: `crate::app::{App, Overlay, View}`, `crate::canvas::Canvas`, `crate::format`, `crate::theme::{rgb, Theme}`, `ergotop_core::reconcile::TxEntry`.
- Produces:
  - `ergotop::ui::draw(f: &mut Frame, app: &mut App, now_ms: u64)`
  - `pub(crate) fn panel(title: String, theme: &Theme) -> Block<'static>`, `pub(crate) fn kv(k: &str, v: String) -> Line<'static>`, `pub(crate) fn util_color(pct: u64, t: &Theme) -> Color`, `pub(crate) fn origin_counts(app: &App) -> Vec<(String, Rgb, usize)>`, `pub(crate) fn origin_text(e: &TxEntry) -> String`
  - `ui::packing::viz_panel(f, area, app: &mut App, now_ms)`
  - Task 6 fills `ui::sources::draw(f, area, &App)` and `ui::overlay::{help, detail}`.

- [ ] **Step 1: Create the stubs for Task 6 modules**

`crates/ergotop/src/ui/sources.rs`:

```rust
//! Sources consistency view (filled in Task 6).
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Paragraph::new("").block(super::panel("SOURCES".into(), &app.theme)), area);
}
```

`crates/ergotop/src/ui/overlay.rs`:

```rust
//! Help and transaction-detail popups (filled in Task 6).
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;

pub fn help(_f: &mut Frame, _area: Rect, _app: &App) {}

pub fn detail(_f: &mut Frame, _area: Rect, _app: &App, _now_ms: u64) {}
```

- [ ] **Step 2: Write the failing tests**

`crates/ergotop/src/ui/mod.rs` (tests first; implementation in Step 4):

```rust
//! Rendering: header, status bar, view dispatch and shared panel helpers.
mod dashboard;
mod overlay;
pub mod packing;
mod sources;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::testkit::{sample_app, specs, NOW};
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
            ],
        );
        insta::assert_snapshot!("dashboard", s);
    }

    #[test]
    fn packing_view_shows_next_block_and_legend() {
        let mut app = sample_app();
        app.view = View::Packing;
        let s = screen(&mut app, 100, 30);
        assert_contains(&s, &["NEXT BLOCK  4 tx", "Contract", "▀"]);
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
            assert_contains(&screen(&mut app, 120, 30), &["no data source"]);
        }
    }

    #[test]
    fn filter_input_shows_in_status_bar() {
        let mut app = sample_app();
        app.filtering = true;
        app.filter = "kuc".into();
        assert_contains(&screen(&mut app, 120, 30), &["/kuc"]);
    }
}
```

Add `pub mod ui;` to `lib.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p ergotop ui::`
Expected: FAIL to compile — `draw`, `panel` not found (and `dashboard.rs`/`packing.rs` missing).

- [ ] **Step 4: Implement `ui/mod.rs`**

Insert below the module declarations, above the tests:

```rust
use std::collections::HashMap;

use ergotop_core::classify::Rgb;
use ergotop_core::reconcile::TxEntry;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Overlay, View};
use crate::format;
use crate::theme::Theme;

const HINTS: &str = " 1 2 3 views  ↑↓ Enter  / filter  s sort  c copy  e explorer  l hex  t theme  ? help  q quit ";

pub fn draw(f: &mut Frame, app: &mut App, now_ms: u64) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(app.theme.bg)), area);
    let [head, body, foot] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);
    header(f, head, app, now_ms);
    match app.view {
        View::Dashboard => dashboard::draw(f, body, app, now_ms),
        View::Packing => packing::draw(f, body, app, now_ms),
        View::Sources => sources::draw(f, body, app),
    }
    status_bar(f, foot, app);
    match app.overlay {
        Overlay::Help => overlay::help(f, area, app),
        Overlay::Detail => overlay::detail(f, area, app, now_ms),
        Overlay::None => {}
    }
}

pub(crate) fn panel(title: String, theme: &Theme) -> Block<'static> {
    Block::bordered()
        .title(Span::styled(format!(" {title} "), Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)))
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
pub(crate) fn origin_counts(app: &App) -> Vec<(String, Rgb, usize)> {
    let mut m: HashMap<String, (Rgb, usize)> = HashMap::new();
    for e in app.rec.pool().values() {
        let c = &e.class.class;
        m.entry(c.name.clone()).or_insert((c.color, 0)).1 += 1;
    }
    let mut v: Vec<(String, Rgb, usize)> = m.into_iter().map(|(n, (c, k))| (n, c, k)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    v
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
            let style = Style::new().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD);
            f.render_widget(Paragraph::new(format!(" {b} ")).style(style), area);
            return;
        }
    }
    let pool_bytes: u64 = app.rec.pool().values().map(|e| e.tx.size as u64).sum();
    let pct = app.utilization_pct();
    let price = app.price.map(|p| format!("${p:.4}")).unwrap_or_else(|| "-".into());
    let line = Line::from(vec![
        Span::styled(" ERGOTOP ", Style::new().fg(t.bg).bg(t.primary).add_modifier(Modifier::BOLD)),
        Span::raw(format!("  Block #{}  ", format::thousands(app.chain_height() as u64))),
        Span::raw(format!("ERG {price}  ")),
        Span::raw(format!("{} tx · {}  ", app.rec.pool().len(), format::bytes(pool_bytes))),
        Span::styled(format!("mempool {pct}% of block"), Style::new().fg(util_color(pct, &t))),
    ]);
    f.render_widget(Paragraph::new(line).style(Style::new().bg(t.panel_bg).fg(t.primary)), area);
}

fn status_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let left = if app.filtering {
        format!(" /{}▏", app.filter)
    } else if let Some((msg, _)) = &app.status {
        format!(" {msg}")
    } else {
        app.source_summary()
    };
    let hint_w = (HINTS.chars().count() as u16).min(area.width / 2);
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(hint_w)]).areas(area);
    f.render_widget(Paragraph::new(left).style(Style::new().bg(t.panel_bg).fg(t.primary)), l);
    f.render_widget(Paragraph::new(HINTS).style(Style::new().bg(t.panel_bg).fg(t.dim)), r);
}
```

- [ ] **Step 5: Implement `ui/packing.rs`**

```rust
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
```

- [ ] **Step 6: Implement `ui/dashboard.rs`**

```rust
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
    let [left, center, right] =
        Layout::horizontal([Constraint::Length(30), Constraint::Min(40), Constraint::Length(36)]).areas(area);
    let [summary_area, blocks_area] = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(left);
    let [viz_area, table_area] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Min(0)]).areas(center);
    let [net_area, origin_area, detail_area] =
        Layout::vertical([Constraint::Length(9), Constraint::Length(10), Constraint::Min(0)]).areas(right);
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
        Line::from(format!("{} / {}", format::bytes(bytes), format::bytes(app.max_block_size() as u64))),
        kv("Total fees", format!("{} ERG", format::fee(fees))),
        kv("Avg fee", dash((n > 0).then(|| format!("{} ERG", format::fee(fees / n))))),
        kv("Avg size", dash((n > 0).then(|| format::bytes(bytes / n)))),
        kv("Largest", dash(pool.iter().map(|e| e.tx.size as u64).max().map(format::bytes))),
        kv(
            "Oldest",
            dash(pool.iter().map(|e| e.first_seen_ms).min().map(|s| format::age(now_ms.saturating_sub(s)))),
        ),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("MEMPOOL".into(), &t)), area);
}

fn blocks(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let lines: Vec<Line> = app
        .rec
        .recent_blocks()
        .into_iter()
        .map(|b| {
            Line::from(format!(
                "#{} {:<9} {:>3}tx {}",
                format::thousands(b.height as u64),
                format::trunc(&app.miner_name(b), 9),
                b.tx_ids.len(),
                format::age(now_ms.saturating_sub(b.timestamp_ms))
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("RECENT BLOCKS".into(), &app.theme)), area);
}

fn tx_table(f: &mut Frame, area: Rect, app: &App, now_ms: u64) {
    let t = app.theme;
    let rows = app.rows();
    let title = format!(
        "TRANSACTIONS {} · sort:{}{}",
        rows.len(),
        app.sort.label(),
        if app.filter.is_empty() { String::new() } else { format!(" · /{}", app.filter) }
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
                Cell::from(format!("{}{}", format::erg(e.metrics.value), if e.metrics.approx { "~" } else { "" })),
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
                dash(i.indexed_height.map(|h| i.full_height.saturating_sub(h).to_string())),
            ));
        }
        None => lines.push(kv("Node", "no node data".into())),
    }
    lines.push(kv("Max block", format::bytes(app.max_block_size() as u64)));
    lines.push(kv("Mempool", format!("{} tx", app.rec.pool().len())));
    f.render_widget(Paragraph::new(lines).block(panel("NETWORK".into(), &app.theme)), area);
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
    f.render_widget(Paragraph::new(lines).block(panel("ORIGINS".into(), &app.theme)), area);
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
                format!("{} ERG{}", format::erg(e.metrics.value), if e.metrics.approx { " ~" } else { "" }),
            ),
            kv("Size", format::bytes(e.tx.size as u64)),
            kv("Seen", format!("{} ago", format::age(now_ms.saturating_sub(e.first_seen_ms)))),
            kv("Sources", e.seen_by.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ")),
            kv("In / Out", format!("{} / {}", e.tx.inputs.len(), e.tx.outputs.len())),
            Line::from(Span::styled("Enter: full detail", Style::new().fg(t.dim))),
        ],
    };
    f.render_widget(Paragraph::new(lines).block(panel("SELECTED".into(), &t)), area);
}
```

- [ ] **Step 6b: Run the content assertions (snapshots not yet created)**

Run: `cargo test -p ergotop ui::`
Expected: `hexagon_mode_is_labelled`, `empty_app_renders_without_sources`, `filter_input_shows_in_status_bar` PASS; `dashboard_shows_all_panels` and `packing_view_shows_next_block_and_legend` pass their `assert_contains` checks and then FAIL only on the missing insta snapshot ("snapshot assertion for 'dashboard' failed"). If any `missing "..."` assertion fails instead, fix the rendering code, not the test.

- [ ] **Step 7: Create and review the snapshots**

Run: `INSTA_UPDATE=always cargo test -p ergotop ui::` (with the Docker wrapper: `DOCKER_EXTRA="-e INSTA_UPDATE=always" .superpowers/sdd/<plan>/cargo test -p ergotop ui::`)
Expected: all 5 tests PASS and `crates/ergotop/src/ui/snapshots/ergotop__ui__tests__dashboard.snap` and `..._packing.snap` exist.

Open both `.snap` files and check by eye: three columns with borders not overlapping; the tx table lists 4 rows starting with `d4000000`; the packing area contains `▀`/`▄` blocks and a dotted capacity line; no text is cut mid-border. If the layout is wrong, fix the code, delete the `.snap` files and repeat this step.

Run: `cargo test -p ergotop`
Expected: all tests PASS without `INSTA_UPDATE`.

- [ ] **Step 8: Commit**

```bash
git add crates/ergotop/src/ui crates/ergotop/src/lib.rs
git commit -m "feat(tui): header, status bar, dashboard and packing views

<trailer>"
```

---

### Task 6: Sources view and overlays

**Files:**
- Modify: `crates/ergotop/src/ui/sources.rs`, `crates/ergotop/src/ui/overlay.rs`, `crates/ergotop/src/ui/mod.rs` (tests)

**Interfaces:**
- Consumes: `App::{rec, source_sel, show_only, selected_entry, address_label, token_name, token_amount, theme}`, `ui::{panel, kv, origin_text}`, `format`.
- Produces: `ui::sources::draw(f, area, &App)`, `ui::overlay::{help(f, area, &App), detail(f, area, &App, now_ms)}`.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `ui/mod.rs`:

```rust
    #[test]
    fn sources_view_compares_sources() {
        let mut app = sample_app();
        app.view = View::Sources;
        let s = screen(&mut app, 130, 30);
        assert_contains(&s, &["SOURCES", "* node-a", "p2p", "Only here", "explorer", "6.0.1"]);
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
        assert_contains(&s, &["KEYS", "Cycle sort", "Toggle hexagon", "Quit"]);
        insta::assert_snapshot!("help", s);
    }

    #[test]
    fn detail_overlay_shows_inputs_outputs_and_names() {
        let mut app = sample_app();
        app.overlay = crate::app::Overlay::Detail;
        app.filter = "kucoin".into();
        let s = screen(&mut app, 120, 34);
        assert_contains(
            &s,
            &["TRANSACTION", "a1000000", "Kucoin", "INPUTS (1)", "OUTPUTS (2)", "fee", "12.00", "c: copy id"],
        );
        insta::assert_snapshot!("detail", s);
    }

    #[test]
    fn tiny_terminal_never_panics() {
        for view in [View::Dashboard, View::Packing, View::Sources] {
            for overlay in [crate::app::Overlay::None, crate::app::Overlay::Help, crate::app::Overlay::Detail] {
                let mut app = sample_app();
                app.view = view;
                app.overlay = overlay;
                app.show_only = true;
                screen(&mut app, 30, 8);
                screen(&mut app, 1, 1);
            }
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop ui::`
Expected: `sources_view_*`, `help_overlay_*`, `detail_overlay_*` FAIL on `missing "..."` (the stubs render nothing); `tiny_terminal_never_panics` may pass already.

- [ ] **Step 3: Implement `ui/sources.rs`**

Replace the file with:

```rust
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
                Cell::from(format!("{}{}", if active { "* " } else { "  " }, format::trunc(&v.id.0, 14))),
                Cell::from(match v.kind {
                    SourceKind::Node => "node",
                    SourceKind::Explorer => "explorer",
                }),
                Cell::from(Span::styled(format::trunc(&status, 28), Style::new().fg(color))),
                Cell::from(v.latency_ms.map(|l| format!("{l}ms")).unwrap_or_else(|| "-".into())),
                Cell::from(v.ids.len().to_string()),
                Cell::from(app.rec.only_in(&v.id).len().to_string()),
                Cell::from(info.map(|i| format::thousands(i.full_height as u64)).unwrap_or_else(|| "-".into())),
                Cell::from(
                    info.and_then(|i| i.indexed_height.map(|h| i.full_height.saturating_sub(h).to_string()))
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(info.map(|i| i.app_version.clone()).unwrap_or_else(|| "-".into())),
            ])
        })
        .collect();
    let header = Row::new(vec!["Source", "Kind", "Status", "Latency", "Txs", "Only here", "Height", "Index lag", "Version"])
        .style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(30),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Min(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(panel("SOURCES  (* active · ↑↓ select · Enter: txs only in source)".into(), &t))
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
                let origin = app.rec.pool().get(id).map(|e| e.class.class.name.clone()).unwrap_or_else(|| "-".into());
                Line::from(format!("{}  {origin}", format::short_id(id)))
            })
            .collect();
        let title = format!("ONLY IN {} ({})", v.id, ids.len());
        f.render_widget(Paragraph::new(lines).block(panel(title, &t)), only_area);
    }
}
```

- [ ] **Step 4: Implement `ui/overlay.rs`**

Replace the file with:

```rust
//! Help and transaction-detail popups.
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::{kv, origin_text, panel};
use crate::app::App;
use crate::format;

const KEYS: [(&str, &str); 12] = [
    ("1 2 3", "Dashboard / Packing / Sources"),
    ("↑ ↓ PgUp PgDn", "Move selection"),
    ("Enter", "Transaction detail (Sources: txs only in source)"),
    ("s", "Cycle sort: fee → value → size → age → origin"),
    ("/", "Filter: name, kind, tx id, >ERG, <ERG"),
    ("Esc", "Clear filter / close"),
    ("c", "Copy tx id"),
    ("e", "Open tx in explorer"),
    ("l", "Toggle hexagon packing"),
    ("t", "Cycle theme"),
    ("?", "Help"),
    ("q", "Quit"),
];

fn centered(area: Rect, w_pct: u16, h_pct: u16) -> Rect {
    let w = area.width * w_pct / 100;
    let h = area.height * h_pct / 100;
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

pub fn help(f: &mut Frame, area: Rect, app: &App) {
    let r = centered(area, 70, 70);
    let lines: Vec<Line> = KEYS.iter().map(|(k, d)| Line::from(format!("{k:<14} {d}"))).collect();
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(panel("KEYS".into(), &app.theme)), r);
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
        kv("Origin", format!("{} ({})", origin_text(e), e.class.class.kind.label())),
        kv("Fee", format!("{} ERG", format::fee(e.metrics.fee))),
        kv(
            "Value",
            format!("{} ERG{}", format::erg(e.metrics.value), if e.metrics.approx { " (approx: inputs unresolved)" } else { "" }),
        ),
        kv("Size", format::bytes(e.tx.size as u64)),
        kv(
            "Seen",
            format!(
                "{} ago by {}",
                format::age(now_ms.saturating_sub(e.first_seen_ms)),
                e.seen_by.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ")
            ),
        ),
        Line::from(""),
        Line::from(Span::styled(format!("INPUTS ({})", e.tx.inputs.len()), heading)),
    ];
    for i in &e.tx.inputs {
        lines.push(match &i.resolved {
            Some(b) => Line::from(format!("  {:<24} {:>16} ERG", app.address_label(&b.address), format::erg(b.value))),
            None => Line::from(format!("  {:<24} (unresolved)", format::short_id(&i.box_id))),
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("OUTPUTS ({})", e.tx.outputs.len()), heading)));
    for o in &e.tx.outputs {
        lines.push(Line::from(format!("  {:<24} {:>16} ERG", app.address_label(&o.address), format::erg(o.value))));
        for tok in &o.tokens {
            lines.push(Line::from(format!("      + {} {}", app.token_amount(tok), app.token_name(&tok.token_id))));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("c: copy id · e: open in explorer · Esc: close", Style::new().fg(t.dim))));
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(panel("TRANSACTION".into(), &t)), r);
}
```

- [ ] **Step 5: Run the content assertions**

Run: `cargo test -p ergotop ui::`
Expected: all non-snapshot assertions PASS; `sources_view_compares_sources`, `help_overlay_lists_keys`, `detail_overlay_shows_inputs_outputs_and_names` FAIL only on their missing snapshots.

- [ ] **Step 6: Create and review the snapshots**

Run: `INSTA_UPDATE=always cargo test -p ergotop ui::`
Expected: PASS; new `sources`, `help`, `detail` `.snap` files. Open them and check: the sources table shows `* node-a` with 4 txs and `Only here` 2, `p2p` with 2 txs; the help popup is centered over the dashboard/packing view with a cleared background; the detail popup shows the full `a1000…` id, one input labelled `Kucoin`, outputs `9guaDYhH…Ym3Rsq` and `fee`.

Run: `cargo test -p ergotop`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/ergotop/src/ui
git commit -m "feat(tui): sources consistency view, help and detail overlays

<trailer>"
```

---

### Task 7: Terminal loop, CLI wiring, logging

**Files:**
- Create: `crates/ergotop/src/tui.rs`
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod tui;`), `crates/ergotop/src/main.rs`

**Interfaces:**
- Consumes: `App`, `Action`, `ui::draw`, `ergotop_core::sources::runtime::{spawn_all, Timing}`, `ergotop_core::config::{cache_dir, AddressesFile, Config}`.
- Produces: `ergotop::tui::{now_ms() -> u64, run(cfg: Config, addrs: AddressesFile, warnings: Vec<String>) -> anyhow::Result<()>}`; CLI flag `--log <file>`.

- [ ] **Step 1: Write the failing test**

`crates/ergotop/src/tui.rs`:

```rust
//! Terminal event loop: source events, key presses and an fps tick; redraw only when dirty.

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
```

Add `pub mod tui;` to `lib.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ergotop tui`
Expected: FAIL to compile — `frame_interval`, `Duration` not found.

- [ ] **Step 3: Implement the loop**

Insert above the tests:

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::execute;
use ergotop_core::config::{cache_dir, AddressesFile, Config};
use ergotop_core::sources::runtime::{spawn_all, Timing};
use futures::StreamExt;

use crate::app::{Action, App};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn frame_interval(fps: u32) -> Duration {
    Duration::from_millis(1000 / u64::from(fps.clamp(1, 120)))
}

pub async fn run(cfg: Config, addrs: AddressesFile, warnings: Vec<String>) -> anyhow::Result<()> {
    let specs = cfg.sources();
    let mut rx = spawn_all(&specs, Timing::default(), cache_dir());
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
```

- [ ] **Step 4: Wire the CLI**

Replace `crates/ergotop/src/main.rs` with:

```rust
use std::path::PathBuf;

use clap::Parser;
use ergotop_core::config::{config_dir, load_from_dir, AddressesFile, Config};

#[derive(Parser)]
#[command(version, about = "Real-time Ergo mempool visualizer")]
struct Args {
    /// Directory containing ergotop.toml and addresses.toml
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print mempool activity as text instead of the TUI
    #[arg(long)]
    headless: bool,
    /// Write logs to this file (nothing is logged to the terminal)
    #[arg(long)]
    log: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if let Some(path) = &args.log {
        let file = std::fs::File::create(path)?;
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    }
    let (mut cfg, addrs, warnings) = match args.config.clone().or_else(config_dir) {
        Some(dir) => load_from_dir(&dir),
        None => (Config::default(), AddressesFile::default(), vec![]),
    };
    cfg.apply_env(std::env::var("ERGO_NODE_URL").ok(), std::env::var("ERGO_API_URL").ok());
    if args.headless {
        for w in &warnings {
            eprintln!("warning: {w}");
        }
        return ergotop::headless::run(cfg, addrs).await;
    }
    ergotop::tui::run(cfg, addrs, warnings).await
}
```

- [ ] **Step 5: Run tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all tests PASS, no clippy warnings.

- [ ] **Step 6: Manual run**

On a machine with a real terminal and Rust (or ask the user), run: `cargo run --release -p ergotop` (add `--config <dir>` for a LAN node).
Expected: the packing view appears within ~2 s with the explorers' txs falling into place; `1`/`2`/`3` switch views; `?` shows keys; `q` exits and restores the terminal (cursor visible, no raw-mode leftovers). With `--log /tmp/ergotop.log` the file receives warnings such as address-book refresh failures.

- [ ] **Step 7: Commit**

```bash
git add crates/ergotop/src/tui.rs crates/ergotop/src/lib.rs crates/ergotop/src/main.rs
git commit -m "feat(tui): terminal event loop, clipboard/explorer actions, --log

<trailer>"
```

---

### Task 8: Frame-time benchmark

**Files:**
- Modify: `crates/ergotop/benches/frame.rs`

**Interfaces:**
- Consumes: `ergotop::viz::{Visualizer, VizItem}`, `ergotop::canvas::Canvas`.
- Produces: criterion benches `relayout_10k` and `render_frame_10k`.

- [ ] **Step 1: Write the benchmark**

Replace `crates/ergotop/benches/frame.rs` with:

```rust
use criterion::{criterion_group, criterion_main, Criterion};
use ergotop::canvas::Canvas;
use ergotop::viz::{Visualizer, VizItem};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const W: u16 = 200;
const H_CELLS: u16 = 50;

fn items(n: usize) -> Vec<VizItem> {
    let mut seed: u64 = 42;
    (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            VizItem {
                id: format!("{i:064}"),
                size_bytes: 200 + (seed >> 33) as u32 % 20_000,
                fee: 1_000_000 + (seed >> 20) % 10_000_000,
                color: Color::Rgb((seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8),
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let txs = items(10_000);
    c.bench_function("relayout_10k", |b| {
        b.iter(|| {
            let mut v = Visualizer::new();
            v.set_size(W, H_CELLS * 2);
            v.relayout(&txs, 1_271_009, false);
        })
    });
    let mut v = Visualizer::new();
    v.set_size(W, H_CELLS * 2);
    v.relayout(&txs, 1_271_009, false);
    let area = Rect::new(0, 0, W, H_CELLS);
    c.bench_function("render_frame_10k", |b| {
        b.iter(|| {
            let mut canvas = Canvas::new(W, H_CELLS * 2);
            v.render(&mut canvas, 0, Color::Gray);
            let mut buf = Buffer::empty(area);
            canvas.render(area, &mut buf);
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
```

- [ ] **Step 2: Run the benchmark**

Run: `cargo bench -p ergotop --bench frame -- --quick`
Expected: `render_frame_10k` mean < 5 ms (spec frame budget). Record both means in the ledger. `relayout_10k` runs only when the pool changes (≤ once per poll), so it is reported, not gated; if it exceeds 50 ms, note it as a finding for the final review.

- [ ] **Step 3: Commit**

```bash
git add crates/ergotop/benches/frame.rs
git commit -m "bench(tui): relayout and frame render with 10k txs

<trailer>"
```

---

## Self-Review Notes

- Spec §4.1 views → Tasks 5–6; §4.2 packing visualizer (half-blocks, size→side, fee-rate block selection, overflow dimmed, falling sand, exact-mined flash+sweep, hexagon toggle) → Tasks 2–3 (+ core packing); §4.3 keys → Task 4; themes → Task 1; §4.4 animation → Task 3 (`Phase` per sprite instead of a separate `Animation` trait: one sprite state machine covers drop, settle, flash and sweep; the header flash is `block_flash_until`). §5 `[ui]` theme/fps/start_view → Tasks 4 and 7. §6 `--log`, panic-safe terminal restore (`ratatui::init` installs a restoring panic hook) → Task 7. §7 snapshot tests → Tasks 5–6; benchmark → Task 8.
- Gravity settle after a mined block happens through `relayout(animate = true)`: remaining sprites keep their current `y` and move to their new lower `target_y`.
