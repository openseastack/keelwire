//! NMEA 2000 Fast-Packet reassembly and fragmentation.

use std::collections::{HashMap, HashSet};

pub const FAST_PACKET_MAX: usize = 223;
/// NMEA 2000 allows up to 750 ms between frames of one fast packet.
pub const DEFAULT_TIMEOUT_US: u64 = 750_000;

/// PGNs that are always fast-packet even without a PGN database.
pub fn builtin_fast_packet(pgn: u32) -> bool {
    matches!(pgn, 126208 | 126464 | 126720 | 126983..=126988 | 126996 | 126998 | 127233 | 127237 | 128275 | 129029 | 129038..=129041 | 129284 | 129285 | 129540 | 129794 | 129809 | 129810 | 130060..=130074 | 130323)
        || (130816..=131071).contains(&pgn)
}

#[derive(Debug)]
struct Session {
    total: usize,
    expected_frames: usize,
    data: Vec<u8>,
    received: u32, // bitmap of frame indexes 0..=31
    last_us: u64,
    first_us: u64,
    priority: u8,
    destination: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FastPacketStats {
    pub completed: u64,
    pub orphans: u64,
    pub timeouts: u64,
    pub restarts: u64,
    pub invalid: u64,
}

/// Completed fast-packet payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub source: u8,
    pub destination: u8,
    pub priority: u8,
    pub pgn: u32,
    pub first_us: u64,
    pub last_us: u64,
    pub data: Vec<u8>,
}

/// Reassembler keyed by (source, pgn, sequence id).
pub struct Reassembler {
    sessions: HashMap<(u8, u32, u8), Session>,
    timeout_us: u64,
    pub stats: FastPacketStats,
    /// Orphaned or timed-out packets per sender.
    pub incomplete_by_source: HashMap<u8, u64>,
    extra_fast: HashSet<u32>,
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT_US)
    }
}

impl Reassembler {
    pub fn new(timeout_us: u64) -> Self {
        Reassembler { sessions: HashMap::new(), timeout_us, stats: FastPacketStats::default(), incomplete_by_source: HashMap::new(), extra_fast: HashSet::new() }
    }

    /// Register PGNs (e.g. from the CANboat DB) to be treated as fast-packet.
    pub fn set_fast_pgns<I: IntoIterator<Item = u32>>(&mut self, pgns: I) {
        self.extra_fast = pgns.into_iter().collect();
    }

    pub fn is_fast(&self, pgn: u32) -> bool {
        self.extra_fast.contains(&pgn) || builtin_fast_packet(pgn)
    }

    pub fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Feed one frame. Returns a payload when the packet completes.
    pub fn push(&mut self, ts_us: u64, priority: u8, pgn: u32, source: u8, destination: u8, frame: &[u8]) -> Option<Assembled> {
        if frame.is_empty() {
            self.stats.invalid += 1;
            return None;
        }
        let seq = frame[0] >> 5;
        let index = (frame[0] & 0x1F) as usize;
        let key = (source, pgn, seq);
        if index == 0 {
            if frame.len() < 2 {
                self.stats.invalid += 1;
                return None;
            }
            let total = frame[1] as usize;
            if total == 0 || total > FAST_PACKET_MAX {
                self.stats.invalid += 1;
                return None;
            }
            if self.sessions.remove(&key).is_some() {
                self.stats.restarts += 1;
            }
            let expected_frames = if total <= 6 { 1 } else { 1 + (total - 6).div_ceil(7) };
            let mut s = Session {
                total,
                expected_frames,
                data: vec![0xFF; total],
                received: 1,
                last_us: ts_us,
                first_us: ts_us,
                priority,
                destination,
            };
            let n = (frame.len() - 2).min(6).min(total);
            s.data[..n].copy_from_slice(&frame[2..2 + n]);
            if expected_frames == 1 {
                self.stats.completed += 1;
                return Some(Assembled { source, destination, priority, pgn, first_us: ts_us, last_us: ts_us, data: s.data });
            }
            self.sessions.insert(key, s);
            return None;
        }
        let Some(s) = self.sessions.get_mut(&key) else {
            self.stats.orphans += 1;
            *self.incomplete_by_source.entry(source).or_default() += 1;
            return None;
        };
        if index >= s.expected_frames {
            self.stats.invalid += 1;
            return None;
        }
        let start = 6 + (index - 1) * 7;
        let n = (frame.len() - 1).min(7).min(s.total.saturating_sub(start));
        s.data[start..start + n].copy_from_slice(&frame[1..1 + n]);
        s.received |= 1 << index;
        s.last_us = ts_us;
        let full_mask: u32 = if s.expected_frames >= 32 { u32::MAX } else { (1u32 << s.expected_frames) - 1 };
        if s.received & full_mask == full_mask {
            let s = self.sessions.remove(&key).unwrap();
            self.stats.completed += 1;
            return Some(Assembled {
                source,
                destination: s.destination,
                priority: s.priority,
                pgn,
                first_us: s.first_us,
                last_us: ts_us,
                data: s.data,
            });
        }
        None
    }

    /// Drop sessions that stalled for longer than the timeout.
    pub fn expire(&mut self, now_us: u64) -> usize {
        let timeout = self.timeout_us;
        let before = self.sessions.len();
        let by_source = &mut self.incomplete_by_source;
        self.sessions.retain(|k, s| {
            let keep = now_us.saturating_sub(s.last_us) <= timeout;
            if !keep {
                *by_source.entry(k.0).or_default() += 1;
            }
            keep
        });
        let dropped = before - self.sessions.len();
        self.stats.timeouts += dropped as u64;
        dropped
    }
}

/// Split a payload into fast-packet frames (each exactly 8 bytes).
pub fn fragment(seq: u8, data: &[u8]) -> Vec<[u8; 8]> {
    assert!(data.len() <= FAST_PACKET_MAX, "fast-packet payload too long");
    let seq = (seq & 0x07) << 5;
    let mut out = Vec::new();
    let mut f = [0xFFu8; 8];
    f[0] = seq;
    f[1] = data.len() as u8;
    let n = data.len().min(6);
    f[2..2 + n].copy_from_slice(&data[..n]);
    out.push(f);
    let mut pos = n;
    let mut idx = 1u8;
    while pos < data.len() {
        let mut f = [0xFFu8; 8];
        f[0] = seq | idx;
        let n = (data.len() - pos).min(7);
        f[1..1 + n].copy_from_slice(&data[pos..pos + n]);
        out.push(f);
        pos += n;
        idx += 1;
    }
    out
}

/// Per-PGN sequence counter for outbound fast packets.
#[derive(Default)]
pub struct SequenceCounter(HashMap<u32, u8>);

impl SequenceCounter {
    pub fn next(&mut self, pgn: u32) -> u8 {
        let e = self.0.entry(pgn).or_insert(0);
        let v = *e;
        *e = (v + 1) & 0x07;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    #[test]
    fn roundtrip_in_order() {
        for n in [1usize, 6, 7, 13, 14, 100, 223] {
            let data = payload(n);
            let frames = fragment(3, &data);
            let mut r = Reassembler::default();
            let mut out = None;
            for (i, f) in frames.iter().enumerate() {
                out = r.push(i as u64 * 1000, 3, 129029, 5, 255, f);
                if i + 1 < frames.len() {
                    assert!(out.is_none());
                }
            }
            assert_eq!(out.unwrap().data, data, "len {n}");
        }
    }

    #[test]
    fn out_of_order_and_interleaved() {
        let a = payload(40);
        let b: Vec<u8> = payload(30).iter().map(|x| x ^ 0x55).collect();
        let fa = fragment(1, &a);
        let fb = fragment(2, &b);
        let mut r = Reassembler::default();
        assert!(r.push(0, 3, 129029, 1, 255, &fa[0]).is_none());
        assert!(r.push(0, 3, 129029, 1, 255, &fb[0]).is_none());
        assert!(r.push(0, 3, 129029, 1, 255, &fa[2]).is_none());
        assert!(r.push(0, 3, 129029, 1, 255, &fa[1]).is_none());
        for f in &fb[1..] {
            let _ = r.push(0, 3, 129029, 1, 255, f);
        }
        let mut last = None;
        for f in &fa[3..] {
            last = r.push(0, 3, 129029, 1, 255, f);
        }
        assert_eq!(last.unwrap().data, a);
        assert_eq!(r.stats.completed, 2);
    }

    #[test]
    fn timeout_and_orphans() {
        let data = payload(50);
        let frames = fragment(0, &data);
        let mut r = Reassembler::new(750_000);
        r.push(0, 3, 129029, 7, 255, &frames[0]);
        r.push(100_000, 3, 129029, 7, 255, &frames[1]);
        assert_eq!(r.expire(500_000), 0);
        assert_eq!(r.expire(900_000), 1);
        assert_eq!(r.stats.timeouts, 1);
        assert!(r.push(950_000, 3, 129029, 7, 255, &frames[2]).is_none());
        assert_eq!(r.stats.orphans, 1);
    }

    #[test]
    fn sequence_counter_wraps() {
        let mut c = SequenceCounter::default();
        let v: Vec<u8> = (0..10).map(|_| c.next(1)).collect();
        assert_eq!(v, vec![0, 1, 2, 3, 4, 5, 6, 7, 0, 1]);
    }
}
