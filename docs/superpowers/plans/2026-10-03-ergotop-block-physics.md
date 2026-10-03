# Physics Block Building Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the visualizer's speed-based falling with time-based tweens: gravity drops with a bounce, sideways slides on repack, a pending state for txs that left the pool, flash → launch for mined txs with a staggered avalanche, fade for dropped txs, a water fill line, and an `m` motion toggle.

**Architecture:** A new pure module `anim.rs` (curves and `Tween::pos(now)`). `viz.rs` is rewritten around per-sprite `State` + `Tween`; every on-screen position is computed from time at render. `App` tracks txs that left the pool and feeds them back to the visualizer as `pending` items until a `Mined`/`Dropped` verdict or a 20 s hold.

**Tech Stack:** Rust, ratatui 0.30 (existing). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-03-ergotop-block-physics-design.md`

## Global Constraints

- Timings (spec §2.2): full-height fall 1,200 ms scaled by `sqrt(distance/height)`, min 120 ms; bounce tail 250 ms, height `min(0.15·fall, 2 px)`; slide 400 ms; flash 400 ms (white/color every 100 ms); launch 700 ms to `height + side`; avalanche delay `400·x/width` ms capped at 400; fade 300 ms; pending hold 20,000 ms; fill glide 500 ms.
- Positions are pure functions of time; a new move always starts from the sprite's current position (no teleporting while motion is on).
- Pending txs keep their packed slot, render at half brightness, and are excluded from block counts and the fill level.
- `[ui] motion` (default `true`) and the `m` key toggle motion; with motion off everything is placed instantly and exits are immediate.
- Frame budget: full Dashboard frame with 10,000 txs mid-animation < 5 ms (release).
- Toolchain on this machine: `.superpowers/cargo` (Docker wrapper); elsewhere plain `cargo`.
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (`<trailer>` below). Branch: `block-physics`.

## Review Focus

1. A relayout that interrupts a running tween must continue from the sprite's current position — pinned in Task 2 (`sideways_repack_slides_without_jumping`).
2. With a node, txs leave the pool before their block arrives; they must wait in place and then launch, not vanish — pinned in Task 3.
3. A pending tx that never gets a verdict must not stay forever — pinned in Task 2 (`expire_pending`) and Task 3 (App tick).
4. Turning motion off mid-animation must leave no half-finished sprites — pinned in Task 2 (`motion_off_...`).
5. 10,000 sprites falling at once must stay within the frame budget — measured in Task 4.

---

## File Structure

```
crates/ergotop/src/anim.rs          NEW  Ease, Tween, timing constants, tween constructors
crates/ergotop/src/viz.rs           REWRITE  State/Tween sprites, pending, mined/dropped/expire, fill line, motion
crates/ergotop/src/lib.rs           + pub mod anim
crates/ergotop/src/app.rs           pending ("leaving") tracking, verdict routing, expiry in tick, `m` key, motion config
crates/ergotop/src/ui/packing.rs    counts exclude pending; render(now, line, water)
crates/ergotop/src/ui/overlay.rs    help lists `m`
crates/ergotop-core/src/config.rs   UiConfig.motion (default true)
crates/ergotop/benches/frame.rs     new API; falling-sprite benches
README.md                           `m` key; `motion` config
```

---

### Task 1: Tween module

**Files:**
- Create: `crates/ergotop/src/anim.rs`
- Modify: `crates/ergotop/src/lib.rs` (add `pub mod anim;`)

**Interfaces:**
- Produces (`ergotop::anim`): constants `FULL_FALL_MS: f32 = 1200.0`, `MIN_FALL_MS: u64 = 120`, `BOUNCE_MS: u64 = 250`, `BOUNCE_MAX_PX: f32 = 2.0`, `BOUNCE_FRACTION: f32 = 0.15`, `SLIDE_MS: u64 = 400`, `LAUNCH_MS: u64 = 700`, `GLIDE_MS: u64 = 500`, `AVALANCHE_MAX_MS: u64 = 400`; `type Point = (f32, f32)`; `enum Ease { Gravity { fall_ms: u64, bounce_px: f32 }, Slide, Launch, Linear }`; `struct Tween { from, to: Point, start_ms, dur_ms: u64, ease: Ease }` (Copy) with `at_rest(Point)`, `end_ms()`, `done(now)`, `pos(now) -> Point`; fns `fall_ms(distance: f32, height: u16) -> u64`, `gravity_drop(from, to, start_ms, height: u16) -> Tween`, `slide(from, to, start_ms) -> Tween`, `launch(from, top: f32, start_ms) -> Tween`, `glide(from: f32, to: f32, start_ms) -> Tween` (y-only, x = 0), `avalanche_delay(x: u16, width: u16) -> u64`.

- [ ] **Step 1: Write the failing tests**

`crates/ergotop/src/anim.rs`:

```rust
//! Time-based tweens: every animated position is a pure function of time.

#[cfg(test)]
mod tests {
    use super::*;

    const H: u16 = 20;

    #[test]
    fn fall_duration_scales_with_sqrt_distance() {
        assert_eq!(fall_ms(20.0, H), 1200);
        assert_eq!(fall_ms(5.0, H), 600);
        assert_eq!(fall_ms(0.01, H), MIN_FALL_MS);
        assert_eq!(fall_ms(5.0, 0), MIN_FALL_MS);
    }

    #[test]
    fn gravity_drop_starts_accelerates_lands_bounces_and_rests() {
        let t = gravity_drop((3.0, 20.0), (3.0, 0.0), 1000, H);
        assert_eq!(t.pos(999), (3.0, 20.0));
        assert_eq!(t.pos(1000), (3.0, 20.0));
        let mid = t.pos(1600).1;
        assert!(20.0 - mid < mid, "second half of the fall covers more ground (mid = {mid})");
        assert_eq!(t.pos(2200), (3.0, 0.0), "lands at the end of the fall");
        let peak = t.pos(2200 + BOUNCE_MS / 2).1;
        assert!((peak - 2.0).abs() < 1e-4, "bounce peak is capped at 2 px (got {peak})");
        assert_eq!(t.end_ms(), 2200 + BOUNCE_MS);
        assert_eq!(t.pos(t.end_ms()), (3.0, 0.0));
        assert!(t.done(t.end_ms()));
    }

    #[test]
    fn small_drops_bounce_proportionally() {
        let t = gravity_drop((0.0, 4.0), (0.0, 0.0), 0, H);
        match t.ease {
            Ease::Gravity { bounce_px, .. } => assert!((bounce_px - 0.6).abs() < 1e-5),
            other => panic!("unexpected ease {other:?}"),
        }
    }

    #[test]
    fn slide_eases_in_and_out_symmetrically() {
        let t = slide((0.0, 0.0), (10.0, 4.0), 0);
        assert!(t.pos(100).0 < 2.5, "slow start");
        assert_eq!(t.pos(200), (5.0, 2.0), "midpoint at half time");
        assert_eq!(t.pos(SLIDE_MS), (10.0, 4.0));
    }

    #[test]
    fn launch_accelerates_off_the_top() {
        let t = launch((2.0, 5.0), 26.0, 0);
        let half = t.pos(LAUNCH_MS / 2).1 - 5.0;
        assert!(half < 26.0 - 5.0 - half, "accelerates upward");
        assert_eq!(t.pos(LAUNCH_MS), (2.0, 26.0));
    }

    #[test]
    fn glide_is_linear_on_y() {
        let t = glide(0.0, 10.0, 100);
        assert_eq!(t.pos(350).1, 5.0);
        assert_eq!(t.pos(100 + GLIDE_MS).1, 10.0);
    }

    #[test]
    fn avalanche_delays_grow_left_to_right() {
        assert_eq!(avalanche_delay(0, 60), 0);
        assert_eq!(avalanche_delay(30, 60), 200);
        assert_eq!(avalanche_delay(60, 60), AVALANCHE_MAX_MS);
        assert_eq!(avalanche_delay(90, 60), AVALANCHE_MAX_MS);
        assert_eq!(avalanche_delay(5, 0), 0);
    }

    #[test]
    fn at_rest_never_moves() {
        let t = Tween::at_rest((1.0, 2.0));
        assert_eq!(t.pos(0), (1.0, 2.0));
        assert_eq!(t.pos(99_999), (1.0, 2.0));
        assert!(t.done(0));
    }
}
```

Add `pub mod anim;` to `crates/ergotop/src/lib.rs` (keep the list alphabetical: `anim` first).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop --lib anim`
Expected: FAIL to compile — `fall_ms`, `gravity_drop`, `Tween`, … not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
use std::f32::consts::PI;

/// A full-height gravity fall takes this long; shorter falls scale with sqrt(distance).
pub const FULL_FALL_MS: f32 = 1_200.0;
pub const MIN_FALL_MS: u64 = 120;
pub const BOUNCE_MS: u64 = 250;
pub const BOUNCE_MAX_PX: f32 = 2.0;
pub const BOUNCE_FRACTION: f32 = 0.15;
pub const SLIDE_MS: u64 = 400;
pub const LAUNCH_MS: u64 = 700;
pub const GLIDE_MS: u64 = 500;
pub const AVALANCHE_MAX_MS: u64 = 400;

pub type Point = (f32, f32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    /// Quadratic ease-in over `fall_ms`, then a sine bounce of `bounce_px` over `BOUNCE_MS`.
    Gravity { fall_ms: u64, bounce_px: f32 },
    /// Cubic ease-in-out.
    Slide,
    /// Quadratic ease-in (accelerating).
    Launch,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: Point,
    pub to: Point,
    pub start_ms: u64,
    pub dur_ms: u64,
    pub ease: Ease,
}

fn lerp(a: Point, b: Point, p: f32) -> Point {
    (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p)
}

impl Tween {
    pub fn at_rest(p: Point) -> Tween {
        Tween { from: p, to: p, start_ms: 0, dur_ms: 0, ease: Ease::Linear }
    }

    pub fn end_ms(&self) -> u64 {
        self.start_ms + self.dur_ms
    }

    pub fn done(&self, now_ms: u64) -> bool {
        now_ms >= self.end_ms()
    }

    pub fn pos(&self, now_ms: u64) -> Point {
        if now_ms <= self.start_ms {
            return self.from;
        }
        if self.done(now_ms) {
            return self.to;
        }
        let e = now_ms - self.start_ms;
        let u = e as f32 / self.dur_ms as f32;
        match self.ease {
            Ease::Gravity { fall_ms, bounce_px } => {
                if e < fall_ms {
                    let v = e as f32 / fall_ms as f32;
                    lerp(self.from, self.to, v * v)
                } else {
                    let b = (e - fall_ms) as f32 / BOUNCE_MS as f32;
                    (self.to.0, self.to.1 + bounce_px * (PI * b).sin())
                }
            }
            Ease::Slide => {
                let p = if u < 0.5 { 4.0 * u * u * u } else { 1.0 - (-2.0 * u + 2.0).powi(3) / 2.0 };
                lerp(self.from, self.to, p)
            }
            Ease::Launch => lerp(self.from, self.to, u * u),
            Ease::Linear => lerp(self.from, self.to, u),
        }
    }
}

pub fn fall_ms(distance: f32, height: u16) -> u64 {
    if height == 0 || distance <= 0.0 {
        return MIN_FALL_MS;
    }
    ((FULL_FALL_MS * (distance / height as f32).sqrt()) as u64).max(MIN_FALL_MS)
}

pub fn gravity_drop(from: Point, to: Point, start_ms: u64, height: u16) -> Tween {
    let d = (from.1 - to.1).abs();
    let fall = fall_ms(d, height);
    let bounce_px = (BOUNCE_FRACTION * d).min(BOUNCE_MAX_PX);
    let tail = if bounce_px > 0.0 { BOUNCE_MS } else { 0 };
    Tween { from, to, start_ms, dur_ms: fall + tail, ease: Ease::Gravity { fall_ms: fall, bounce_px } }
}

pub fn slide(from: Point, to: Point, start_ms: u64) -> Tween {
    Tween { from, to, start_ms, dur_ms: SLIDE_MS, ease: Ease::Slide }
}

pub fn launch(from: Point, top: f32, start_ms: u64) -> Tween {
    Tween { from, to: (from.0, top), start_ms, dur_ms: LAUNCH_MS, ease: Ease::Launch }
}

/// A y-only glide (x stays 0), used for the fill-line level.
pub fn glide(from: f32, to: f32, start_ms: u64) -> Tween {
    Tween { from: (0.0, from), to: (0.0, to), start_ms, dur_ms: GLIDE_MS, ease: Ease::Linear }
}

pub fn avalanche_delay(x: u16, width: u16) -> u64 {
    if width == 0 {
        return 0;
    }
    (AVALANCHE_MAX_MS * x as u64 / width as u64).min(AVALANCHE_MAX_MS)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop --lib anim`
Expected: 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop/src/anim.rs crates/ergotop/src/lib.rs
git commit -m "feat(viz): time-based tween curves for block physics

<trailer>"
```

---

### Task 2: Visualizer rewrite

**Files:**
- Rewrite: `crates/ergotop/src/viz.rs`
- Modify: `crates/ergotop/src/app.rs` (adapt to the new visualizer API only), `crates/ergotop/src/ui/packing.rs`, `crates/ergotop/benches/frame.rs`

**Interfaces:**
- Consumes: `crate::anim::{self, Point, Tween}`, `ergotop_core::packing::*`, `crate::canvas::Canvas`.
- Produces (`ergotop::viz`):
  - consts `MAX_SIDE: u16 = 6`, `FLASH_MS: u64 = 400`, `FADE_MS: u64 = 300`, `PENDING_HOLD_MS: u64 = 20_000`
  - `pub fn dim(Color) -> Color` (half brightness for `Color::Rgb`)
  - `pub enum State { Active, Pending { since_ms: u64 }, Flashing { until_ms: u64 }, Launching, Fading { until_ms: u64 } }`
  - `pub struct Sprite { pub id, pub x: u16, pub target_y: u16, pub side: u16, pub size_bytes: u32, pub fee: u64, pub color: Color, pub region: Region, pub state: State, pub tween: Tween }` with `pos(now) -> Point`
  - `pub struct VizItem { pub id, pub size_bytes: u32, pub fee: u64, pub color: Color, pub pending: bool }` (Clone)
  - `pub struct Visualizer { pub width, pub height: u16, pub shape: Shape, pub motion: bool, pub last: PackResult, pub block_count: usize, pub block_bytes: u64, .. }` with `new()`, `block_height()`, `params(capacity)`, `set_size(w, h) -> bool`, `set_motion(bool)`, `sprite(&str)`, `placed() -> impl Iterator<&Sprite>` (packed sprites only), `sprites()` (packed + leaving), `fill_level(now) -> f32`, `relayout(items, capacity, now_ms, instant: bool)`, `on_mined(ids, now)`, `on_dropped(ids, now)`, `expire_pending(now) -> Vec<TxId>`, `tick(now) -> bool`, `render(canvas, now, line: Color, water: Color)`
- App adaptation in this task: `App::relayout(animate)` passes `instant = !animate` and `pending: false`; `Update::Dropped` calls `viz.on_dropped`; `App::tick` calls `viz.tick(now_ms)`; the `last_tick_ms` field is removed.

- [ ] **Step 1: Write the new `viz.rs` tests (replace the file)**

Replace `crates/ergotop/src/viz.rs` with only this header and test module (implementation follows in Step 3):

```rust
//! Next-block visualizer with time-based physics: txs fall in, slide when repacked,
//! wait dimmed while pending, launch when mined and fade when dropped.

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgb(200, 0, 0);
    const BLUE: Color = Color::Rgb(0, 0, 200);
    const CAP: u32 = 1_000_000;

    fn item(id: &str, size: u32, fee: u64) -> VizItem {
        VizItem { id: id.into(), size_bytes: size, fee, color: RED, pending: false }
    }

    fn pending(id: &str, size: u32, fee: u64) -> VizItem {
        VizItem { pending: true, ..item(id, size, fee) }
    }

    fn ids(v: &[&str]) -> Vec<TxId> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn instant_relayout_places_sprites_at_rest() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], CAP, 0, true);
        assert_eq!(v.sprite("a").unwrap().pos(0), (0.0, 0.0));
        assert!(!v.tick(0));
    }

    #[test]
    fn new_sprites_fall_from_above_and_land() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], CAP, 0, false);
        let a = v.sprite("a").unwrap().clone();
        assert_eq!(a.pos(0), (0.0, 20.0));
        assert!(v.tick(600));
        let y = a.pos(600).1;
        assert!(y > 0.0 && y < 20.0);
        assert_eq!(a.pos(1200), (0.0, 0.0));
        assert!(!v.tick(2000));
    }

    #[test]
    fn sideways_repack_slides_without_jumping() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 400, 4000)], CAP, 0, false);
        let before = v.sprite("a").unwrap().pos(600);
        v.relayout(&[item("b", 20_000, 10_000_000), item("a", 400, 4000)], CAP, 600, false);
        let a = v.sprite("a").unwrap().clone();
        let after = a.pos(600);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3, "{before:?} vs {after:?}");
        assert_eq!((a.x, a.target_y), (6, 0));
        let mid = a.pos(800).0;
        assert!(mid > 0.0 && mid < 6.0, "slides through x={mid}");
        assert_eq!(a.pos(1000), (6.0, 0.0));
    }

    #[test]
    fn pending_sprites_keep_their_slot_and_are_not_counted() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 400, 4000), item("b", 400, 3000)], CAP, 0, true);
        let slot = (v.sprite("b").unwrap().x, v.sprite("b").unwrap().target_y);
        v.relayout(&[item("a", 400, 4000), pending("b", 400, 3000)], CAP, 10, false);
        let b = v.sprite("b").unwrap();
        assert_eq!((b.x, b.target_y), slot);
        assert_eq!(b.state, State::Pending { since_ms: 10 });
        assert_eq!((v.block_count, v.block_bytes), (1, 400));
    }

    #[test]
    fn mined_sprites_flash_then_launch_and_leave() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], CAP, 0, true);
        v.on_mined(&ids(&["a"]), 1_000);
        assert!(v.sprite("a").is_none());
        assert!(v.sprites().any(|s| matches!(s.state, State::Flashing { .. })));
        v.tick(1_000 + FLASH_MS);
        assert!(v.sprites().any(|s| s.state == State::Launching));
        assert!(v.tick(1_000 + FLASH_MS + 100));
        v.tick(1_000 + FLASH_MS + crate::anim::LAUNCH_MS);
        assert_eq!(v.sprites().count(), 1);
    }

    #[test]
    fn avalanche_starts_after_the_flash_left_columns_first() {
        let mut v = Visualizer::new();
        v.set_size(6, 40);
        v.relayout(
            &[item("a", 20_000, 10_000_000), item("c", 400, 4000), item("d", 400, 4000)],
            CAP,
            0,
            true,
        );
        assert_eq!((v.sprite("c").unwrap().x, v.sprite("c").unwrap().target_y), (0, 6));
        assert_eq!((v.sprite("d").unwrap().x, v.sprite("d").unwrap().target_y), (2, 6));
        v.on_mined(&ids(&["a"]), 1_000);
        v.relayout(&[item("c", 400, 4000), item("d", 400, 4000)], CAP, 1_000, false);
        let c = v.sprite("c").unwrap().clone();
        let d = v.sprite("d").unwrap().clone();
        assert_eq!(c.pos(1_399).1, 6.0, "waits for the flash");
        assert!(c.pos(1_450).1 < 6.0, "left column falls first");
        assert_eq!(d.pos(1_450).1, 6.0, "right column still waiting");
        assert_eq!(c.pos(5_000), (0.0, 0.0));
        assert_eq!(d.pos(5_000), (2.0, 0.0));
    }

    #[test]
    fn dropped_and_expired_pending_sprites_fade_out() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], CAP, 0, true);
        v.on_dropped(&ids(&["a"]), 1_000);
        assert!(v.sprites().any(|s| matches!(s.state, State::Fading { .. })));
        v.tick(1_299);
        assert_eq!(v.sprites().count(), 2);
        v.tick(1_300);
        assert_eq!(v.sprites().count(), 1);

        v.relayout(&[pending("b", 1000, 10)], CAP, 2_000, false);
        assert!(v.expire_pending(2_000 + PENDING_HOLD_MS - 1).is_empty());
        assert_eq!(v.expire_pending(2_000 + PENDING_HOLD_MS), ids(&["b"]));
        assert!(v.sprite("b").is_none());
        assert!(v.sprites().any(|s| matches!(s.state, State::Fading { .. })));
    }

    #[test]
    fn fill_line_glides_to_block_fullness() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        v.relayout(&[item("a", 600, 10)], 1_000, 0, true);
        assert!(close(v.fill_level(0), 9.0), "600/1000 of a 15 px block");
        v.relayout(&[item("a", 600, 10), item("b", 300, 10)], 1_000, 100, false);
        assert!(close(v.fill_level(100), 9.0));
        assert!(close(v.fill_level(350), 11.25));
        assert!(close(v.fill_level(600), 13.5));
    }

    #[test]
    fn motion_off_places_instantly_and_skips_exits() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10)], CAP, 0, false);
        v.set_motion(false);
        assert_eq!(v.sprite("a").unwrap().pos(1), (0.0, 0.0), "running fall snapped to target");
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], CAP, 10, false);
        let b = v.sprite("b").unwrap();
        assert_eq!(b.pos(10), (b.x as f32, b.target_y as f32));
        v.on_mined(&ids(&["a"]), 20);
        v.on_dropped(&ids(&["b"]), 20);
        assert_eq!(v.sprites().count(), 0);
        assert!(!v.tick(20));
    }

    #[test]
    fn render_colors_block_overflow_pending_and_water() {
        let mut v = Visualizer::new();
        v.set_size(10, 8);
        v.relayout(&[item("a", 400, 4000), item("b", 400, 1)], 500, 0, true);
        let a = v.sprite("a").unwrap().clone();
        let b = v.sprite("b").unwrap().clone();
        assert_eq!(a.region, Region::Block);
        assert_eq!(b.region, Region::Overflow);
        let mut c = Canvas::new(10, 8);
        v.render(&mut c, 0, Color::Gray, BLUE);
        assert_eq!(c.get(a.x, a.target_y), Some(RED));
        assert_eq!(c.get(b.x, b.target_y), Some(Color::Rgb(100, 0, 0)));
        assert_eq!(c.get(5, 3), Some(BLUE), "water line at level 4.8 → rows 3-4");

        v.relayout(&[pending("a", 400, 4000)], 500, 0, true);
        let mut c = Canvas::new(10, 8);
        v.render(&mut c, 0, Color::Gray, BLUE);
        assert_eq!(c.get(0, 0), Some(Color::Rgb(100, 0, 0)), "pending renders dimmed");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop --lib viz`
Expected: FAIL to compile — `Visualizer`, `VizItem`, `State`, … not found (and `app.rs`/`packing.rs` errors from the removed API).

- [ ] **Step 3: Implement `viz.rs`**

Insert below the module doc comment, above the tests:

```rust
use std::collections::HashMap;

use ergotop_core::model::TxId;
use ergotop_core::packing::{pack, PackItem, PackParams, PackResult, Region, Shape};
use ratatui::style::Color;

use crate::anim::{self, Point, Tween};
use crate::canvas::Canvas;

pub const MAX_SIDE: u16 = 6;
pub const FLASH_MS: u64 = 400;
pub const FADE_MS: u64 = 300;
/// Longest a tx that left the pool waits (dimmed, in place) for a mined/dropped verdict.
pub const PENDING_HOLD_MS: u64 = 20_000;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Active,
    Pending { since_ms: u64 },
    Flashing { until_ms: u64 },
    Launching,
    Fading { until_ms: u64 },
}

#[derive(Clone, Debug)]
pub struct Sprite {
    pub id: TxId,
    pub x: u16,
    pub target_y: u16,
    pub side: u16,
    pub size_bytes: u32,
    pub fee: u64,
    pub color: Color,
    pub region: Region,
    pub state: State,
    pub tween: Tween,
}

impl Sprite {
    pub fn pos(&self, now_ms: u64) -> Point {
        self.tween.pos(now_ms)
    }
}

#[derive(Clone, Debug)]
pub struct VizItem {
    pub id: TxId,
    pub size_bytes: u32,
    pub fee: u64,
    pub color: Color,
    /// Left the mempool; waiting for a mined/dropped verdict. Keeps its slot, not counted.
    pub pending: bool,
}

pub struct Visualizer {
    pub width: u16,
    pub height: u16,
    pub shape: Shape,
    pub motion: bool,
    pub last: PackResult,
    /// Next-block txs and bytes, excluding pending ones.
    pub block_count: usize,
    pub block_bytes: u64,
    sprites: HashMap<TxId, Sprite>,
    leaving: Vec<Sprite>,
    fill: Tween,
    avalanche_at: Option<u64>,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn dim(c: Color) -> Color {
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
            motion: true,
            last: PackResult::default(),
            block_count: 0,
            block_bytes: 0,
            sprites: HashMap::new(),
            leaving: Vec::new(),
            fill: Tween::at_rest((0.0, 0.0)),
            avalanche_at: None,
        }
    }

    pub fn block_height(&self) -> u16 {
        self.height * 3 / 4
    }

    pub fn params(&self, capacity: u32) -> PackParams {
        let block_height = self.block_height();
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

    /// Turning motion off finishes every running animation immediately.
    pub fn set_motion(&mut self, on: bool) {
        self.motion = on;
        if !on {
            for s in self.sprites.values_mut() {
                s.tween = Tween::at_rest((s.x as f32, s.target_y as f32));
            }
            self.leaving.clear();
            self.fill = Tween::at_rest(self.fill.to);
            self.avalanche_at = None;
        }
    }

    pub fn sprite(&self, id: &str) -> Option<&Sprite> {
        self.sprites.get(id)
    }

    /// Sprites that hold a packed slot (active and pending).
    pub fn placed(&self) -> impl Iterator<Item = &Sprite> {
        self.sprites.values()
    }

    /// Placed sprites plus those flashing, launching or fading out.
    pub fn sprites(&self) -> impl Iterator<Item = &Sprite> {
        self.sprites.values().chain(self.leaving.iter())
    }

    pub fn fill_level(&self, now_ms: u64) -> f32 {
        self.fill.pos(now_ms).1
    }

    pub fn relayout(&mut self, items: &[VizItem], capacity: u32, now_ms: u64, instant: bool) {
        let pack_items: Vec<PackItem> = items
            .iter()
            .map(|i| PackItem { id: i.id.clone(), size_bytes: i.size_bytes, fee: i.fee })
            .collect();
        let by_id: HashMap<&str, &VizItem> = items.iter().map(|i| (i.id.as_str(), i)).collect();
        let result = pack(&pack_items, &self.params(capacity));
        let animate = self.motion && !instant;
        if instant {
            self.leaving.clear();
            self.avalanche_at = None;
        }
        let settle_at = self.avalanche_at.take().filter(|t| *t > now_ms);
        let top = self.height as f32;
        let mut next = HashMap::with_capacity(result.placed.len());
        let (mut count, mut bytes) = (0usize, 0u64);
        for p in &result.placed {
            let Some(item) = by_id.get(p.id.as_str()) else {
                continue;
            };
            if p.region == Region::Block && !item.pending {
                count += 1;
                bytes += item.size_bytes as u64;
            }
            let to = (p.x as f32, p.y as f32);
            let prev = self.sprites.remove(&p.id);
            let tween = match &prev {
                _ if !animate => Tween::at_rest(to),
                None => anim::gravity_drop((to.0, top), to, now_ms, self.height),
                Some(s) if (s.x, s.target_y) == (p.x, p.y) => s.tween,
                Some(s) => {
                    let cur = s.pos(now_ms);
                    let start = match settle_at {
                        Some(t) => t + anim::avalanche_delay(p.x, self.width),
                        None => now_ms,
                    };
                    if s.x == p.x && to.1 <= cur.1 {
                        anim::gravity_drop(cur, to, start, self.height)
                    } else {
                        anim::slide(cur, to, start)
                    }
                }
            };
            let state = if item.pending {
                match prev.as_ref().map(|s| &s.state) {
                    Some(State::Pending { since_ms }) => State::Pending { since_ms: *since_ms },
                    _ => State::Pending { since_ms: now_ms },
                }
            } else {
                State::Active
            };
            next.insert(
                p.id.clone(),
                Sprite {
                    id: p.id.clone(),
                    x: p.x,
                    target_y: p.y,
                    side: p.side,
                    size_bytes: item.size_bytes,
                    fee: item.fee,
                    color: item.color,
                    region: p.region,
                    state,
                    tween,
                },
            );
        }
        self.sprites = next;
        self.last = result;
        self.block_count = count;
        self.block_bytes = bytes;

        let block_h = self.block_height() as f32;
        let level = if capacity == 0 { 0.0 } else { (bytes as f32 / capacity as f32 * block_h).min(block_h) };
        if instant || !animate {
            self.fill = Tween::at_rest((0.0, level));
        } else if level != self.fill.to.1 {
            self.fill = anim::glide(self.fill_level(now_ms), level, now_ms);
        }
    }

    fn exit(&mut self, ids: &[TxId], now_ms: u64, state: impl Fn(u64) -> State, hold_ms: u64) {
        let mut any = false;
        for id in ids {
            if let Some(mut s) = self.sprites.remove(id) {
                any = true;
                if self.motion {
                    s.tween = Tween::at_rest(s.pos(now_ms));
                    s.state = state(now_ms + hold_ms);
                    self.leaving.push(s);
                }
            }
        }
        if any && self.motion {
            self.avalanche_at = Some(now_ms + hold_ms);
        }
    }

    pub fn on_mined(&mut self, ids: &[TxId], now_ms: u64) {
        self.exit(ids, now_ms, |until_ms| State::Flashing { until_ms }, FLASH_MS);
    }

    pub fn on_dropped(&mut self, ids: &[TxId], now_ms: u64) {
        self.exit(ids, now_ms, |until_ms| State::Fading { until_ms }, FADE_MS);
    }

    /// Pending sprites past `PENDING_HOLD_MS` fade out; returns their ids.
    pub fn expire_pending(&mut self, now_ms: u64) -> Vec<TxId> {
        let expired: Vec<TxId> = self
            .sprites
            .values()
            .filter(|s| matches!(s.state, State::Pending { since_ms } if now_ms.saturating_sub(since_ms) >= PENDING_HOLD_MS))
            .map(|s| s.id.clone())
            .collect();
        if !expired.is_empty() {
            self.on_dropped(&expired, now_ms);
        }
        expired
    }

    /// Advances flash → launch and removes finished exits; true while anything is moving.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let top = self.height as f32;
        for s in self.leaving.iter_mut() {
            if let State::Flashing { until_ms } = s.state {
                if now_ms >= until_ms {
                    let from = s.pos(now_ms);
                    s.tween = anim::launch(from, top + s.side as f32, until_ms);
                    s.state = State::Launching;
                }
            }
        }
        self.leaving.retain(|s| match s.state {
            State::Launching => !s.tween.done(now_ms),
            State::Fading { until_ms } => now_ms < until_ms,
            _ => true,
        });
        !self.leaving.is_empty() || !self.fill.done(now_ms) || self.sprites.values().any(|s| !s.tween.done(now_ms))
    }

    pub fn render(&self, canvas: &mut Canvas, now_ms: u64, line: Color, water: Color) {
        let block_height = self.block_height();
        if block_height < self.height {
            for x in (0..self.width).step_by(2) {
                canvas.set(x as i32, block_height as i32, line);
            }
        }
        let level = self.fill_level(now_ms).round() as i32;
        if level > 0 {
            let phase = now_ms / 150;
            for x in 0..self.width {
                let wave = ((x as u64 + phase) % 2) as i32;
                canvas.set(x as i32, (level - 1 - wave).max(0), water);
            }
        }
        for s in self.sprites() {
            let color = match s.state {
                State::Flashing { .. } if (now_ms / 100).is_multiple_of(2) => Color::White,
                State::Pending { .. } | State::Fading { .. } => dim(s.color),
                _ if s.region == Region::Overflow => dim(s.color),
                _ => s.color,
            };
            let (x, y) = s.pos(now_ms);
            canvas.fill(x.round() as i32, y.round() as i32, s.side, s.side, color);
        }
    }
}
```

- [ ] **Step 4: Adapt `app.rs`, `ui/packing.rs` and the bench to the new API**

In `crates/ergotop/src/app.rs`:

1. Delete the field line `    last_tick_ms: u64,` and the initializer line `            last_tick_ms: 0,`.
2. In `on_updates`, replace `self.viz.forget(ids);` with `self.viz.on_dropped(ids, now_ms);`.
3. In `relayout`, add `pending: false,` to the `VizItem { .. }` literal and replace
   `self.viz.relayout(&items, capacity, animate, self.clock_ms);` with
   `self.viz.relayout(&items, capacity, self.clock_ms, !animate);`.
4. In `tick`, replace the block from `let dt = if self.last_tick_ms == 0 {` through `let animating = self.viz.tick(dt, now_ms);` with
   `let animating = self.viz.tick(now_ms);`.
5. In the tests, replace `crate::viz::Phase::Flashing { .. }` with `crate::viz::State::Flashing { .. }`.

In `crates/ergotop/src/ui/packing.rs`:

1. Replace `r.block_count,` with `app.viz.block_count,` and `format::bytes(r.block_bytes),` with `format::bytes(app.viz.block_bytes),` in the title, and the percentage line `let pct = r.block_bytes * 100 / max.max(1);` with `let pct = app.viz.block_bytes * 100 / max.max(1);`.
2. Replace `app.viz.render(&mut canvas, now_ms, t.dim);` with `app.viz.render(&mut canvas, now_ms, t.dim, crate::viz::dim(t.accent));`.

In `crates/ergotop/benches/frame.rs`: add `pending: false,` to the `VizItem { .. }` literal; replace every `relayout(&txs, 1_271_009, false, 0)` with `relayout(&txs, 1_271_009, 0, true)`; replace `v.render(&mut canvas, 0, Color::Gray);` with `v.render(&mut canvas, 0, Color::Gray, Color::Blue);`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ergotop`
Expected: 10 viz tests PASS and all other ergotop tests PASS (the UI snapshots are unchanged: the sample mempool fills < 1% of the block, so no water line is drawn).

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/ergotop
git commit -m "feat(viz): tween-based block physics (drop, slide, pending, launch, fade, water line)

<trailer>"
```

---

### Task 3: Pending tracking in App, motion toggle and config

**Files:**
- Modify: `crates/ergotop/src/app.rs`, `crates/ergotop/src/ui/overlay.rs`, `crates/ergotop-core/src/config.rs`, `README.md`

**Interfaces:**
- Consumes: `Visualizer::{placed, on_mined, on_dropped, expire_pending, set_motion, motion}`, `VizItem.pending`, `viz::PENDING_HOLD_MS`.
- Produces: `UiConfig.motion: bool` (default `true`); `App` field `leaving: HashMap<TxId, VizItem>`; `m` key → toggles motion with status `"Motion on"` / `"Motion off"`.

- [ ] **Step 1: Write the failing tests**

In `crates/ergotop-core/src/config.rs` tests, append to `defaults_use_local_node_and_both_explorers`:

```rust
        assert!(Config::default().ui.motion);
```

and in `parses_full_config` add `motion = false` under `[ui]` in the TOML string and append:

```rust
        assert!(!cfg.ui.motion);
```

In `crates/ergotop/src/app.rs` tests, replace the body of `mined_txs_flash_when_the_mempool_drops_them_before_the_block_arrives` with:

```rust
        let mut app = sample_app();
        let e5 = tx("e5", 500, WALLET, vec![bx(CONTRACT, 2_000_000_000), bx(FEE_ADDRESS, 1_200_000)]);
        app.on_source_event(node_mempool(vec![tid("b2"), tid("c3"), tid("d4"), tid("e5")], vec![e5]), NOW + 1_000);
        let a1 = app.viz.sprite(&tid("a1")).expect("a1 waits in place");
        assert!(matches!(a1.state, crate::viz::State::Pending { .. }));
        app.on_source_event(block_with(1_886_102, vec!["cb2".into(), tid("a1")]), NOW + 3_000);
        assert!(app.viz.sprite(&tid("a1")).is_none());
        let flashing = app
            .viz
            .sprites()
            .filter(|s| matches!(s.state, crate::viz::State::Flashing { .. }))
            .count();
        assert_eq!(flashing, 1, "a1 launches from where it was built");
```

and append these tests to the module:

```rust
    #[test]
    fn pending_txs_fade_when_the_reconciler_drops_them() {
        let mut app = sample_app();
        let rest = vec![tid("b2"), tid("c3"), tid("d4")];
        app.on_source_event(node_mempool(rest.clone(), vec![]), NOW + 1_000);
        let a1 = app.viz.sprite(&tid("a1")).expect("a1 still holds its slot");
        assert!(
            matches!(a1.state, crate::viz::State::Pending { .. }),
            "a removal with no other change still marks the tx pending"
        );
        app.on_source_event(node_mempool(rest, vec![]), NOW + 17_000);
        assert!(app.viz.sprite(&tid("a1")).is_none());
        assert!(app.viz.sprites().any(|s| matches!(s.state, crate::viz::State::Fading { .. })));
    }

    #[test]
    fn pending_txs_expire_after_the_hold() {
        let mut app = sample_app();
        app.on_source_event(node_mempool(vec![tid("b2"), tid("c3"), tid("d4")], vec![]), NOW + 1_000);
        app.tick(NOW + 1_000 + crate::viz::PENDING_HOLD_MS - 1);
        assert!(app.viz.sprite(&tid("a1")).is_some());
        app.tick(NOW + 1_000 + crate::viz::PENDING_HOLD_MS);
        assert!(app.viz.sprite(&tid("a1")).is_none());
    }

    #[test]
    fn m_toggles_motion_and_config_sets_the_default() {
        let mut app = sample_app();
        assert!(app.viz.motion);
        app.on_key(key(KeyCode::Char('m')), NOW);
        assert!(!app.viz.motion);
        assert_eq!(app.status.as_ref().unwrap().0, "Motion off");
        app.on_key(key(KeyCode::Char('m')), NOW);
        assert!(app.viz.motion);
        let ui = ergotop_core::config::UiConfig { motion: false, ..Default::default() };
        assert!(!App::new(&specs(), Default::default(), &ui).viz.motion);
    }
```

In `crates/ergotop/src/ui/mod.rs` test `help_overlay_lists_keys`, add `"Toggle motion"` to the `assert_contains` list.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --workspace`
Expected: FAIL to compile — `UiConfig` has no field `motion`. After Step 3a compiles, the app/help tests FAIL on their assertions (`a1 waits in place`, `Motion off`, missing `Toggle motion`).

- [ ] **Step 3: Implement**

3a. `crates/ergotop-core/src/config.rs` — add the field and default:

```rust
pub struct UiConfig {
    pub theme: String,
    pub fps: u32,
    pub start_view: String,
    pub motion: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { theme: "neon-green".into(), fps: 30, start_view: "packing".into(), motion: true }
    }
}
```

(Keep the existing `#[derive(Debug, Clone, Deserialize, PartialEq)]` and `#[serde(default)]` attributes.)

3b. `crates/ergotop/src/app.rs`:

- Add `TxId` to the `ergotop_core::model` import if not present (it is, from the selection fix) and add the field after `pub viz: Visualizer,`:

```rust
    /// Txs that left the pool but still occupy their slot until mined, dropped or expired.
    leaving: HashMap<TxId, VizItem>,
```

  with initializer `leaving: HashMap::new(),`.
- In `App::new`, after building the struct (assign it to `let mut app = App { .. };`), call `app.viz.set_motion(ui.motion);` and return `app`.
- In `on_updates`, make the two arms:

```rust
                Update::Dropped(ids) => {
                    for id in ids {
                        self.leaving.remove(id);
                    }
                    self.viz.on_dropped(ids, now_ms);
                    changed = true;
                }
                Update::Mined { tx_ids, .. } => {
                    for id in tx_ids {
                        self.leaving.remove(id);
                    }
                    self.viz.on_mined(tx_ids, now_ms);
                    changed = true;
                }
```

- At the end of the `for u in updates` loop in `on_updates` (before `if changed {`), add — a poll that only removes txs emits no update, but those sprites must still become pending:

```rust
        if !changed {
            let pool = self.rec.pool();
            changed = self
                .viz
                .placed()
                .any(|s| s.state == crate::viz::State::Active && !pool.contains_key(&s.id));
        }
```

- Replace `relayout` with:

```rust
    pub fn relayout(&mut self, animate: bool) {
        let pool = self.rec.pool();
        if animate {
            for s in self.viz.placed() {
                if !pool.contains_key(&s.id) && !self.leaving.contains_key(&s.id) {
                    self.leaving.insert(
                        s.id.clone(),
                        VizItem { id: s.id.clone(), size_bytes: s.size_bytes, fee: s.fee, color: s.color, pending: true },
                    );
                }
            }
        } else {
            self.leaving.clear();
        }
        self.leaving.retain(|id, _| !pool.contains_key(id));
        let mut items: Vec<VizItem> = pool
            .values()
            .map(|e| VizItem {
                id: e.tx.id.clone(),
                size_bytes: e.tx.size,
                fee: e.metrics.fee,
                color: rgb(e.class.class.color),
                pending: false,
            })
            .collect();
        items.extend(self.leaving.values().cloned());
        let capacity = self.max_block_size();
        self.viz.relayout(&items, capacity, self.clock_ms, !animate);
    }
```

- In `tick`, before `let animating = self.viz.tick(now_ms);` insert:

```rust
        let expired = self.viz.expire_pending(now_ms);
        if !expired.is_empty() {
            for id in &expired {
                self.leaving.remove(id);
            }
            self.relayout(true);
        }
```

- In `on_key`, add before the `KeyCode::Char('?')` arm:

```rust
            KeyCode::Char('m') => {
                let on = !self.viz.motion;
                self.viz.set_motion(on);
                self.set_status(format!("Motion {}", if on { "on" } else { "off" }), now_ms);
                Action::None
            }
```

3c. `crates/ergotop/src/ui/overlay.rs` — change `const KEYS: [(&str, &str); 12]` to `13` and insert `("m", "Toggle motion"),` before `("?", "Help"),`.

3d. `README.md` — in the keys table insert a row `| \`m\` | Toggle motion (animations) |` after the `t` row, and in the `ergotop.toml` example add `motion = true                        # false: no animations` under `start_view`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: all PASS except the `help` snapshot (new line) — regenerate it: `INSTA_UPDATE=always cargo test -p ergotop ui::` (Docker: `DOCKER_EXTRA="-e INSTA_UPDATE=always" .superpowers/cargo test -p ergotop ui::`), open `crates/ergotop/src/ui/snapshots/ergotop__ui__tests__help.snap` and confirm the only change is the added `m  Toggle motion` line; then `cargo test --workspace` PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates README.md
git commit -m "feat(tui): pending txs wait in place for their verdict; m toggles motion

<trailer>"
```

---

### Task 4: Benchmarks with everything in motion

**Files:**
- Modify: `crates/ergotop/benches/frame.rs`

**Interfaces:** consumes `Visualizer::relayout(.., instant=false)`, `App::relayout(true)`, `ui::draw`.

- [ ] **Step 1: Add the falling benches**

In `bench`, after the `render_frame_10k` bench, add:

```rust
    let mut falling = Visualizer::new();
    falling.set_size(W, H_CELLS * 2);
    falling.relayout(&txs, 1_271_009, 0, false);
    c.bench_function("render_frame_10k_falling", |b| {
        b.iter(|| {
            let mut canvas = Canvas::new(W, H_CELLS * 2);
            falling.render(&mut canvas, 300, Color::Gray, Color::Blue);
            let mut buf = Buffer::empty(area);
            canvas.render(area, &mut buf);
        })
    });
```

In `bench_dashboard`, after the existing `dashboard_frame_10k` bench, add:

```rust
    // Restart every sprite as a fresh fall (spawned at clock 1_000) and draw mid-fall.
    app.viz.relayout(&[], 1_271_009, 1_000, true);
    app.relayout(true);
    c.bench_function("dashboard_frame_10k_falling", |b| {
        b.iter(|| {
            term.draw(|f| ergotop::ui::draw(f, &mut app, 1_300)).unwrap();
        })
    });
```

- [ ] **Step 2: Run the benchmarks**

Run: `cargo bench -p ergotop --bench frame -- --quick`
Expected: `dashboard_frame_10k_falling` < 5 ms and `render_frame_10k_falling` < 5 ms. Record all four results in the ledger.

- [ ] **Step 3: Commit**

```bash
git add crates/ergotop/benches/frame.rs
git commit -m "bench(viz): frames with 10k sprites mid-fall

<trailer>"
```

---

## Self-Review Notes

- Spec §2 (tweens, curves, timings) → Task 1; §3.1–3.3 (states, events, fill line) → Task 2; pending tracking / verdict routing / hold expiry → Tasks 2–3; §3.4 motion toggle + config + help → Task 3; §5 testing → each task, benchmark → Task 4.
- `instant = true` (resync, resize, hexagon toggle) clears pending/exits in the visualizer and `App` clears `leaving` (spec §3.2 last row).
