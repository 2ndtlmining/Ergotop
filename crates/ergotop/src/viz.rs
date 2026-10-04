//! Next-block visualizer with time-based physics: txs fall in, slide when repacked,
//! wait dimmed while pending, launch when mined and fade when dropped.
use std::collections::HashMap;

use ergotop_core::model::TxId;
use ergotop_core::packing::{pack, shape_bounds, PackItem, PackParams, PackResult, Region, Shape};
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
    /// Active txs that did not fit on screen (pending ones are not counted).
    pub not_shown: usize,
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
            not_shown: 0,
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
            .map(|i| PackItem {
                id: i.id.clone(),
                size_bytes: i.size_bytes,
                fee: i.fee,
            })
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
                // New arrivals wait (above the block, invisible) until a busy slot has cleared.
                None => {
                    anim::gravity_drop((to.0, top), to, settle_at.unwrap_or(now_ms), self.height)
                }
                Some(s) if (s.x, s.target_y) == (p.x, p.y) => s.tween,
                Some(s) => {
                    let cur = s.pos(now_ms);
                    // Only resting sprites wait for the avalanche; one still moving keeps moving.
                    let start = match settle_at {
                        Some(t) if s.tween.done(now_ms) => {
                            t + anim::avalanche_delay(p.x, self.width)
                        }
                        _ => now_ms,
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
                    Some(State::Pending { since_ms }) => State::Pending {
                        since_ms: *since_ms,
                    },
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
        self.not_shown = items
            .iter()
            .filter(|i| !i.pending && !self.sprites.contains_key(&i.id))
            .count();

        let block_h = self.block_height() as f32;
        let level = if capacity == 0 {
            0.0
        } else {
            (bytes as f32 / capacity as f32 * block_h).min(block_h)
        };
        if instant || !animate {
            self.fill = Tween::at_rest((0.0, level));
        } else if level != self.fill.to.1 {
            self.fill = anim::glide(self.fill_level(now_ms), level, now_ms);
        }
    }

    /// Moves `ids` out of their slots into an exit animation lasting `hold_ms`; the rest of the
    /// block settles `settle_ms` from now (the latest pending exit wins).
    fn exit(
        &mut self,
        ids: &[TxId],
        now_ms: u64,
        state: impl Fn(u64) -> State,
        hold_ms: u64,
        settle_ms: u64,
    ) {
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
            let at = now_ms + settle_ms;
            self.avalanche_at = Some(self.avalanche_at.map_or(at, |a| a.max(at)));
        }
    }

    pub fn on_mined(&mut self, ids: &[TxId], now_ms: u64) {
        self.exit(
            ids,
            now_ms,
            |until_ms| State::Flashing { until_ms },
            FLASH_MS,
            FLASH_MS + anim::LAUNCH_MS,
        );
    }

    pub fn on_dropped(&mut self, ids: &[TxId], now_ms: u64) {
        self.exit(
            ids,
            now_ms,
            |until_ms| State::Fading { until_ms },
            FADE_MS,
            FADE_MS,
        );
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
        !self.leaving.is_empty()
            || !self.fill.done(now_ms)
            || self.sprites.values().any(|s| !s.tween.done(now_ms))
    }

    pub fn render(&self, canvas: &mut Canvas, now_ms: u64, line: Color, water: Color) {
        let block_height = self.block_height();
        if block_height < self.height {
            for x in (0..self.width).step_by(2) {
                canvas.set(x as i32, block_height as i32, line);
            }
        }
        let (lo, hi) = shape_bounds(self.width, block_height, self.shape);
        if self.shape == Shape::Hexagon {
            // The slanted sides, so the shape shows even when only its bottom tip is filled.
            for x in 0..self.width as usize {
                if lo[x] > 0 {
                    canvas.set(x as i32, lo[x] as i32 - 1, line);
                }
                if hi[x] < block_height {
                    canvas.set(x as i32, hi[x] as i32, line);
                }
            }
        }
        let level = self.fill_level(now_ms).round() as i32;
        if level >= 2 {
            let phase = now_ms / 150;
            for x in 0..self.width {
                let wave = ((x as u64 + phase) % 2) as i32;
                let y = (level - 1 - wave).max(0);
                // Water stays inside the block's shape.
                let col = x as usize;
                if y >= i32::from(lo[col]) && y < i32::from(hi[col]) {
                    canvas.set(x as i32, y, water);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgb(200, 0, 0);
    const BLUE: Color = Color::Rgb(0, 0, 200);
    const CAP: u32 = 1_000_000;

    fn item(id: &str, size: u32, fee: u64) -> VizItem {
        VizItem {
            id: id.into(),
            size_bytes: size,
            fee,
            color: RED,
            pending: false,
        }
    }

    fn pending(id: &str, size: u32, fee: u64) -> VizItem {
        VizItem {
            pending: true,
            ..item(id, size, fee)
        }
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
        v.relayout(
            &[item("b", 20_000, 10_000_000), item("a", 400, 4000)],
            CAP,
            600,
            false,
        );
        let a = v.sprite("a").unwrap().clone();
        let after = a.pos(600);
        assert!(
            (before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3,
            "{before:?} vs {after:?}"
        );
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
        v.relayout(
            &[item("a", 400, 4000), pending("b", 400, 3000)],
            CAP,
            10,
            false,
        );
        let b = v.sprite("b").unwrap();
        assert_eq!((b.x, b.target_y), slot);
        assert_eq!(b.state, State::Pending { since_ms: 10 });
        assert_eq!((v.block_count, v.block_bytes), (1, 400));
    }

    #[test]
    fn not_shown_counts_only_active_txs() {
        let mut v = Visualizer::new();
        v.set_size(4, 4);
        // Far more txs than a 4x4 grid holds; unplaced pending ones must not count.
        let ids: Vec<String> = (0..40).map(|i| format!("t{i:02}")).collect();
        let items: Vec<VizItem> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                if i % 2 == 0 {
                    item(id, 300, 10)
                } else {
                    pending(id, 300, 10)
                }
            })
            .collect();
        v.relayout(&items, CAP, 0, true);
        let hidden = |pend: bool| {
            items
                .iter()
                .filter(|i| i.pending == pend && v.sprite(&i.id).is_none())
                .count()
        };
        assert!(hidden(true) > 0 && hidden(false) > 0);
        assert_eq!(v.last.not_shown, hidden(true) + hidden(false));
        assert_eq!(v.not_shown, hidden(false));
    }

    #[test]
    fn mined_sprites_flash_then_launch_and_leave() {
        let mut v = Visualizer::new();
        v.set_size(20, 20);
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], CAP, 0, true);
        v.on_mined(&ids(&["a"]), 1_000);
        assert!(v.sprite("a").is_none());
        assert!(v
            .sprites()
            .any(|s| matches!(s.state, State::Flashing { .. })));
        v.tick(1_000 + FLASH_MS);
        assert!(v.sprites().any(|s| s.state == State::Launching));
        assert!(v.tick(1_000 + FLASH_MS + 100));
        v.tick(1_000 + FLASH_MS + crate::anim::LAUNCH_MS);
        assert_eq!(v.sprites().count(), 1);
    }

    #[test]
    fn avalanche_starts_after_the_launch_left_columns_first() {
        let mut v = Visualizer::new();
        v.set_size(6, 40);
        v.relayout(
            &[
                item("a", 20_000, 10_000_000),
                item("c", 400, 4000),
                item("d", 400, 4000),
            ],
            CAP,
            0,
            true,
        );
        v.on_mined(&ids(&["a"]), 1_000);
        v.relayout(
            &[item("c", 400, 4000), item("d", 400, 4000)],
            CAP,
            1_000,
            false,
        );
        let c = v.sprite("c").unwrap().clone();
        let d = v.sprite("d").unwrap().clone();
        let settle = 1_000 + FLASH_MS + crate::anim::LAUNCH_MS;
        assert_eq!(c.pos(settle - 1).1, 6.0, "waits for flash and launch");
        assert!(c.pos(settle + 50).1 < 6.0, "left column falls first");
        assert_eq!(d.pos(settle + 50).1, 6.0, "right column still waiting");
        assert_eq!(c.pos(9_000), (0.0, 0.0));
        assert_eq!(d.pos(9_000), (2.0, 0.0));
    }

    #[test]
    fn a_sprite_still_falling_keeps_falling_when_a_block_lands() {
        let mut v = Visualizer::new();
        v.set_size(6, 40);
        v.relayout(&[item("a", 20_000, 10_000_000)], CAP, 0, true);
        v.relayout(
            &[item("a", 20_000, 10_000_000), item("n", 400, 4000)],
            CAP,
            1_000,
            false,
        );
        v.on_mined(&ids(&["a"]), 1_300);
        v.relayout(&[item("n", 400, 4000)], CAP, 1_300, false);
        let n = v.sprite("n").unwrap();
        assert!(
            n.pos(1_400).1 < n.pos(1_300).1,
            "no mid-air hang while the avalanche waits"
        );
    }

    #[test]
    fn a_drop_in_the_same_batch_does_not_shorten_the_mined_wait() {
        let mut v = Visualizer::new();
        v.set_size(6, 40);
        v.relayout(
            &[
                item("a", 20_000, 10_000_000),
                item("b", 400, 5000),
                item("c", 400, 4000),
            ],
            CAP,
            0,
            true,
        );
        v.on_mined(&ids(&["a"]), 1_000);
        v.on_dropped(&ids(&["b"]), 1_000);
        v.relayout(&[item("c", 400, 4000)], CAP, 1_000, false);
        let c = v.sprite("c").unwrap();
        let settle = 1_000 + FLASH_MS + crate::anim::LAUNCH_MS;
        assert_eq!(
            c.pos(settle - 1),
            c.pos(1_000),
            "still waiting while a flashes and launches"
        );
    }

    #[test]
    fn new_arrivals_wait_for_the_launch_before_falling() {
        let mut v = Visualizer::new();
        v.set_size(6, 40);
        v.relayout(&[item("a", 20_000, 10_000_000)], CAP, 0, true);
        v.on_mined(&ids(&["a"]), 1_000);
        v.relayout(&[item("n", 400, 4000)], CAP, 1_000, false);
        let n = v.sprite("n").unwrap();
        assert_eq!(
            n.pos(1_500).1,
            40.0,
            "held above the block while the slot is busy"
        );
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
        assert_eq!(
            v.sprite("a").unwrap().pos(1),
            (0.0, 0.0),
            "running fall snapped to target"
        );
        v.relayout(&[item("a", 1000, 10), item("b", 1000, 10)], CAP, 10, false);
        let b = v.sprite("b").unwrap();
        assert_eq!(b.pos(10), (b.x as f32, b.target_y as f32));
        v.on_mined(&ids(&["a"]), 20);
        v.on_dropped(&ids(&["b"]), 20);
        assert_eq!(v.sprites().count(), 0);
        assert!(!v.tick(20));
    }

    #[test]
    fn hexagon_mode_draws_its_outline() {
        let mut v = Visualizer::new();
        v.set_size(40, 40);
        v.relayout(&[], CAP, 0, true);
        let mut c = Canvas::new(40, 40);
        v.render(&mut c, 0, Color::Gray, BLUE);
        let lit = |c: &Canvas| {
            (0..40)
                .flat_map(|x| (0..30).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y).is_some())
                .count()
        };
        assert_eq!(lit(&c), 0, "rect mode: no outline inside the block");
        v.shape = Shape::Hexagon;
        v.relayout(&[], CAP, 0, true);
        let mut c = Canvas::new(40, 40);
        v.render(&mut c, 0, Color::Gray, BLUE);
        let (lo, hi) = shape_bounds(40, v.block_height(), Shape::Hexagon);
        assert_eq!(c.get(0, lo[0] - 1), Some(Color::Gray), "lower-left edge");
        assert_eq!(c.get(0, hi[0]), Some(Color::Gray), "upper-left edge");
        assert_eq!(c.get(20, 0), None, "the flat bottom is the floor");

        // Water is clipped to the hexagon: at its level the corner columns stay dry.
        v.relayout(&[item("a", 50_000, 10)], 1_000_000, 0, true);
        let mut c = Canvas::new(40, 40);
        v.render(&mut c, 0, Color::Gray, BLUE);
        assert!(
            (0..40).any(|x| (0..30).any(|y| c.get(x, y) == Some(BLUE))),
            "water drawn"
        );
        for y in 0..lo[0] {
            assert_ne!(
                c.get(0, y),
                Some(BLUE),
                "no water outside the shape at row {y}"
            );
        }
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
        assert_eq!(
            c.get(5, 3),
            Some(BLUE),
            "water line at level 4.8 → rows 3-4"
        );

        v.relayout(&[pending("a", 400, 4000)], 500, 0, true);
        let mut c = Canvas::new(10, 8);
        v.render(&mut c, 0, Color::Gray, BLUE);
        assert_eq!(
            c.get(0, 0),
            Some(Color::Rgb(100, 0, 0)),
            "pending renders dimmed"
        );
    }
}
