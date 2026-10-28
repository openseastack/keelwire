//! Process-wide monotonic clock shared by all drivers and the engine.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<(Instant, u64)> = OnceLock::new();

fn start() -> &'static (Instant, u64) {
    START.get_or_init(|| (Instant::now(), SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)))
}

/// Microseconds since process start (monotonic).
pub fn now_us() -> u64 {
    start().0.elapsed().as_micros() as u64
}

/// Convert an engine timestamp to Unix microseconds.
pub fn to_unix_us(ts_us: u64) -> u64 {
    start().1 + ts_us
}

pub fn unix_now_us() -> u64 {
    to_unix_us(now_us())
}
