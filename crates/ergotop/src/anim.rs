//! Time-based tweens: every animated position is a pure function of time.
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
