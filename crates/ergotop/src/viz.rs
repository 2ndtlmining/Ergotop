//! Animated packing visualizer: sprites fall into their packed positions.
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
