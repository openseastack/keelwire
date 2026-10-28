//! Cumulative traffic counters per sender and per (PGN, sender), since a
//! resettable start. Feeds the Devices table (by device and by PGN).

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceTotals {
    pub source: u8,
    pub frames: u64,
    /// CAN data bytes (frame payloads, including fast-packet headers).
    pub bytes: u64,
    pub messages: u64,
    /// Multi-frame messages from this sender that never completed.
    pub incomplete: u64,
    pub first_us: u64,
    pub last_us: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PgnTotals {
    pub pgn: u32,
    pub source: u8,
    pub messages: u64,
    pub frames: u64,
    /// Message payload bytes.
    pub bytes: u64,
    /// Frames of the most recent message (1 = single frame).
    pub frames_per_msg: u8,
    pub first_us: u64,
    pub last_us: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrafficSnapshot {
    /// Engine time (µs) when counting started.
    pub since_us: u64,
    pub sources: Vec<SourceTotals>,
    pub pgns: Vec<PgnTotals>,
}

#[derive(Default)]
pub struct Traffic {
    since_us: Option<u64>,
    sources: HashMap<u8, SourceTotals>,
    pgns: HashMap<(u32, u8), PgnTotals>,
    /// Incomplete counts reported by the reassembler at the last reset.
    incomplete_base: HashMap<u8, u64>,
}

impl Traffic {
    pub fn new() -> Self {
        Self::default()
    }

    fn start(&mut self, ts_us: u64) {
        if self.since_us.is_none() {
            self.since_us = Some(ts_us);
        }
    }

    pub fn record_frame(&mut self, ts_us: u64, source: u8, dlc: u8) {
        self.start(ts_us);
        let e = self.sources.entry(source).or_insert(SourceTotals { source, first_us: ts_us, ..Default::default() });
        e.frames += 1;
        e.bytes += dlc.min(8) as u64;
        e.last_us = e.last_us.max(ts_us);
    }

    /// `frames` is 0 for messages a gateway delivered already reassembled.
    pub fn record_message(&mut self, ts_us: u64, pgn: u32, source: u8, frames: u8, len: usize) {
        self.start(ts_us);
        let s = self.sources.entry(source).or_insert(SourceTotals { source, first_us: ts_us, ..Default::default() });
        s.messages += 1;
        s.last_us = s.last_us.max(ts_us);
        let frames = if frames == 0 { if len <= 8 { 1 } else { 1 + (len - 6).div_ceil(7) as u8 } } else { frames };
        let p = self.pgns.entry((pgn, source)).or_insert(PgnTotals { pgn, source, first_us: ts_us, ..Default::default() });
        p.messages += 1;
        p.frames += frames as u64;
        p.bytes += len as u64;
        p.frames_per_msg = frames;
        p.last_us = p.last_us.max(ts_us);
    }

    /// Absolute per-source incomplete counts from the reassembler.
    pub fn set_incomplete(&mut self, counts: &HashMap<u8, u64>) {
        for (src, n) in counts {
            let base = self.incomplete_base.get(src).copied().unwrap_or(0);
            if let Some(s) = self.sources.get_mut(src) {
                s.incomplete = n.saturating_sub(base);
            }
        }
    }

    /// Start counting again from `now_us`.
    pub fn reset(&mut self, now_us: u64, incomplete_now: &HashMap<u8, u64>) {
        self.since_us = Some(now_us);
        self.sources.clear();
        self.pgns.clear();
        self.incomplete_base = incomplete_now.clone();
    }

    pub fn snapshot(&self) -> TrafficSnapshot {
        let mut sources: Vec<SourceTotals> = self.sources.values().copied().collect();
        sources.sort_by_key(|s| s.source);
        let mut pgns: Vec<PgnTotals> = self.pgns.values().copied().collect();
        pgns.sort_by_key(|p| (p.pgn, p.source));
        TrafficSnapshot { since_us: self.since_us.unwrap_or(0), sources, pgns }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_reset() {
        let mut t = Traffic::new();
        t.record_frame(1_000, 5, 8);
        t.record_message(1_000, 127250, 5, 1, 8);
        for i in 0..4 {
            t.record_frame(2_000 + i, 6, 8);
        }
        t.record_message(2_004, 129029, 6, 4, 25);
        t.record_message(3_000, 129029, 6, 0, 43); // reassembled by a gateway
        let mut inc = HashMap::new();
        inc.insert(6u8, 2u64);
        t.set_incomplete(&inc);
        let s = t.snapshot();
        assert_eq!(s.since_us, 1_000);
        let s6 = s.sources.iter().find(|x| x.source == 6).unwrap();
        assert_eq!((s6.frames, s6.bytes, s6.messages, s6.incomplete), (4, 32, 2, 2));
        let p = s.pgns.iter().find(|p| p.pgn == 129029).unwrap();
        assert_eq!((p.messages, p.frames, p.bytes, p.frames_per_msg), (2, 4 + 7, 68, 7));
        t.reset(5_000, &inc);
        t.record_frame(6_000, 6, 8);
        t.set_incomplete(&inc);
        let s = t.snapshot();
        assert_eq!(s.since_us, 5_000);
        assert_eq!(s.sources[0].incomplete, 0);
        assert!(s.pgns.is_empty());
    }
}
