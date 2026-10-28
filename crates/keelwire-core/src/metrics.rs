//! Bus statistics: frame rate, bus load, error rate, per-device and per-PGN rates.

use std::collections::{HashMap, VecDeque};

pub const BITRATE: f64 = 250_000.0;

/// Approximate on-wire bits for an extended CAN frame with `dlc` data bytes,
/// including ~10 % bit stuffing and 3-bit interframe space.
pub fn frame_bits(dlc: u8) -> u32 {
    let raw = 67 + 8 * dlc.min(8) as u32;
    raw + raw / 10 + 3
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Second {
    pub t: u64, // seconds since start
    pub frames: u32,
    pub messages: u32,
    pub errors: u32,
    pub bits: u64,
    pub load_pct: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MetricsSnapshot {
    pub frames_per_sec: f64,
    pub messages_per_sec: f64,
    pub errors_per_sec: f64,
    pub load_pct: f64,
    pub peak_load_pct: f64,
    pub total_frames: u64,
    pub total_messages: u64,
    pub total_errors: u64,
    pub dropped: u64,
    pub fp_timeouts: u64,
    pub fp_orphans: u64,
    pub tp_aborts: u64,
    pub history: Vec<Second>,
    /// source address -> (frames/s, % of bus)
    pub per_source: Vec<(u8, f64, f64)>,
    /// (pgn, source) -> msgs/s
    pub per_pgn: Vec<(u32, u8, f64)>,
}

pub struct Metrics {
    current: Second,
    history: VecDeque<Second>,
    per_source_cur: HashMap<u8, (u32, u64)>,
    per_source_last: HashMap<u8, (u32, u64)>,
    per_pgn_cur: HashMap<(u32, u8), u32>,
    per_pgn_rate: HashMap<(u32, u8), f64>,
    pub total_frames: u64,
    pub total_messages: u64,
    pub total_errors: u64,
    pub dropped: u64,
    pub fp_timeouts: u64,
    pub fp_orphans: u64,
    pub tp_aborts: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            current: Second::default(),
            history: VecDeque::with_capacity(61),
            per_source_cur: HashMap::new(),
            per_source_last: HashMap::new(),
            per_pgn_cur: HashMap::new(),
            per_pgn_rate: HashMap::new(),
            total_frames: 0,
            total_messages: 0,
            total_errors: 0,
            dropped: 0,
            fp_timeouts: 0,
            fp_orphans: 0,
            tp_aborts: 0,
        }
    }

    /// The most recently completed second, if any.
    pub fn last_second(&self) -> Option<&Second> {
        self.history.back()
    }

    fn roll(&mut self, ts_us: u64) {
        let sec = ts_us / 1_000_000;
        while sec > self.current.t {
            let mut done = std::mem::take(&mut self.current);
            let t = done.t;
            done.load_pct = (done.bits as f64 / BITRATE * 100.0).min(100.0);
            self.history.push_back(done);
            while self.history.len() > 60 {
                self.history.pop_front();
            }
            self.per_source_last = std::mem::take(&mut self.per_source_cur);
            // exponential smoothing of per-PGN rates
            for (k, r) in self.per_pgn_rate.iter_mut() {
                let c = *self.per_pgn_cur.get(k).unwrap_or(&0) as f64;
                *r = *r * 0.7 + c * 0.3;
            }
            for (k, c) in self.per_pgn_cur.drain() {
                self.per_pgn_rate.entry(k).or_insert(c as f64);
            }
            self.per_pgn_rate.retain(|_, r| *r > 0.01);
            self.current.t = if sec - t > 60 { sec } else { t + 1 };
        }
    }

    pub fn record_frame(&mut self, ts_us: u64, source: u8, dlc: u8) {
        self.roll(ts_us);
        let bits = frame_bits(dlc) as u64;
        self.current.frames += 1;
        self.current.bits += bits;
        self.total_frames += 1;
        let e = self.per_source_cur.entry(source).or_default();
        e.0 += 1;
        e.1 += bits;
    }

    pub fn record_message(&mut self, ts_us: u64, pgn: u32, source: u8) {
        self.roll(ts_us);
        self.current.messages += 1;
        self.total_messages += 1;
        *self.per_pgn_cur.entry((pgn, source)).or_default() += 1;
    }

    pub fn record_error(&mut self, ts_us: u64) {
        self.roll(ts_us);
        self.current.errors += 1;
        self.total_errors += 1;
    }

    pub fn pgn_rate(&self, pgn: u32, source: u8) -> f64 {
        *self.per_pgn_rate.get(&(pgn, source)).unwrap_or(&0.0)
    }

    pub fn snapshot(&mut self, now_us: u64) -> MetricsSnapshot {
        self.roll(now_us);
        let last = self.history.back().cloned().unwrap_or_default();
        let mut per_source: Vec<(u8, f64, f64)> = self.per_source_last.iter().map(|(s, (f, b))| (*s, *f as f64, *b as f64 / BITRATE * 100.0)).collect();
        per_source.sort_by_key(|x| x.0);
        let mut per_pgn: Vec<(u32, u8, f64)> = self.per_pgn_rate.iter().map(|((p, s), r)| (*p, *s, (*r * 100.0).round() / 100.0)).collect();
        per_pgn.sort_by_key(|a| (a.0, a.1));
        MetricsSnapshot {
            frames_per_sec: last.frames as f64,
            messages_per_sec: last.messages as f64,
            errors_per_sec: last.errors as f64,
            load_pct: last.load_pct,
            peak_load_pct: self.history.iter().map(|s| s.load_pct).fold(0.0, f64::max),
            total_frames: self.total_frames,
            total_messages: self.total_messages,
            total_errors: self.total_errors,
            dropped: self.dropped,
            fp_timeouts: self.fp_timeouts,
            fp_orphans: self.fp_orphans,
            tp_aborts: self.tp_aborts,
            history: self.history.iter().cloned().collect(),
            per_source,
            per_pgn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_computation() {
        let mut m = Metrics::new();
        // 1000 frames of 8 bytes in second 0
        for i in 0..1000u64 {
            m.record_frame(i * 900, 5, 8);
        }
        let s = m.snapshot(1_000_001);
        assert_eq!(s.frames_per_sec, 1000.0);
        let expected = frame_bits(8) as f64 * 1000.0 / BITRATE * 100.0;
        assert!((s.load_pct - expected).abs() < 1e-9);
        assert_eq!(s.per_source.len(), 1);
        assert!((s.per_source[0].2 - expected).abs() < 1e-6);
    }
}
