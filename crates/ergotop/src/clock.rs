//! App time in Unix ms that never runs backwards.
//!
//! The wall clock is read once; after that time advances with the monotonic clock,
//! so an NTP correction or manual clock change cannot rewind animations, expiries or ages.
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn anchor() -> &'static (u64, Instant) {
    static ANCHOR: OnceLock<(u64, Instant)> = OnceLock::new();
    ANCHOR.get_or_init(|| {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        (wall, Instant::now())
    })
}

pub fn now_ms() -> u64 {
    let (wall, start) = anchor();
    wall + start.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_wall_time_and_never_decreases() {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let a = now_ms();
        assert!(a.abs_diff(wall) < 5_000);
        let mut prev = a;
        for _ in 0..1_000 {
            let t = now_ms();
            assert!(t >= prev);
            prev = t;
        }
    }
}
