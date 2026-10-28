//! Packet history ring buffer and differential byte tracking.

use std::collections::{HashMap, VecDeque};

/// Entry stored in the ring buffer: one complete N2K message (a single
/// frame, or a reassembled fast-packet / TP payload).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PacketRecord {
    pub seq: u64,
    pub timestamp_us: u64,
    pub priority: u8,
    pub pgn: u32,
    pub source: u8,
    pub destination: u8,
    pub data: Vec<u8>,
    /// Number of CAN frames that made up this message (0 = gateway-assembled).
    pub frames: u8,
    /// True when this message was transmitted by us.
    pub tx: bool,
    /// Per-byte change versus the previous message with the same key (-1/0/+1).
    pub diff: Vec<i8>,
}

/// Fixed-capacity circular buffer with monotonically increasing sequence numbers.
pub struct RingBuffer {
    buf: VecDeque<PacketRecord>,
    capacity: usize,
    next_seq: u64,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        RingBuffer { buf: VecDeque::with_capacity(capacity.min(1 << 20)), capacity: capacity.max(1), next_seq: 1 }
    }

    pub fn push(&mut self, mut rec: PacketRecord) -> u64 {
        rec.seq = self.next_seq;
        self.next_seq += 1;
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        let seq = rec.seq;
        self.buf.push_back(rec);
        seq
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn first_seq(&self) -> Option<u64> {
        self.buf.front().map(|r| r.seq)
    }

    pub fn last_seq(&self) -> Option<u64> {
        self.buf.back().map(|r| r.seq)
    }

    /// Records with seq > `after`, at most `limit`.
    pub fn after(&self, after: u64, limit: usize) -> Vec<PacketRecord> {
        let Some(first) = self.first_seq() else { return vec![] };
        let start = if after < first { 0 } else { (after - first + 1) as usize };
        self.buf.iter().skip(start).take(limit).cloned().collect()
    }

    /// The last `n` records.
    pub fn tail(&self, n: usize) -> Vec<PacketRecord> {
        let skip = self.buf.len().saturating_sub(n);
        self.buf.iter().skip(skip).cloned().collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PacketRecord> {
        self.buf.iter()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Per-message-key previous payload; produces -1/0/+1 per byte.
#[derive(Default)]
pub struct DiffCache {
    last: HashMap<(u32, u8, u64), Vec<u8>>,
}

/// Key extension: for PGNs whose first byte is an instance (most data PGNs)
/// we key on it so interleaved instances don't flicker. Callers may pass 0.
pub fn diff_key_extra(pgn: u32, data: &[u8]) -> u64 {
    match pgn {
        127501 | 127502 | 127505 | 127508 | 127488 | 127489 | 130312 | 130316 | 127506 | 127507 => *data.first().unwrap_or(&0) as u64,
        126720 | 130816..=131071 | 65280..=65535 => {
            // proprietary: header + first discriminating byte(s)
            u64::from_le_bytes([
                *data.first().unwrap_or(&0),
                *data.get(1).unwrap_or(&0),
                *data.get(2).unwrap_or(&0),
                *data.get(3).unwrap_or(&0),
                *data.get(6).unwrap_or(&0),
                0,
                0,
                0,
            ])
        }
        _ => 0,
    }
}

impl DiffCache {
    pub fn diff(&mut self, pgn: u32, source: u8, data: &[u8]) -> Vec<i8> {
        let key = (pgn, source, diff_key_extra(pgn, data));
        let out = match self.last.get(&key) {
            Some(prev) => data
                .iter()
                .enumerate()
                .map(|(i, b)| match prev.get(i) {
                    Some(p) if b > p => 1,
                    Some(p) if b < p => -1,
                    Some(_) => 0,
                    None => 1,
                })
                .collect(),
            None => vec![0; data.len()],
        };
        self.last.insert(key, data.to_vec());
        out
    }

    pub fn clear(&mut self) {
        self.last.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pgn: u32) -> PacketRecord {
        PacketRecord { seq: 0, timestamp_us: 0, priority: 2, pgn, source: 1, destination: 255, data: vec![1], frames: 1, tx: false, diff: vec![] }
    }

    #[test]
    fn ring_wraps_and_pages() {
        let mut r = RingBuffer::new(3);
        for i in 0..5 {
            r.push(rec(i));
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.first_seq(), Some(3));
        assert_eq!(r.after(0, 10).len(), 3);
        assert_eq!(r.after(3, 10).iter().map(|x| x.seq).collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!(r.after(5, 10).len(), 0);
        assert_eq!(r.tail(2).len(), 2);
    }

    #[test]
    fn diffs() {
        let mut d = DiffCache::default();
        assert_eq!(d.diff(127250, 1, &[1, 2, 3]), vec![0, 0, 0]);
        assert_eq!(d.diff(127250, 1, &[1, 3, 2]), vec![0, 1, -1]);
        // different instance is tracked separately
        assert_eq!(d.diff(127505, 1, &[0, 5]), vec![0, 0]);
        assert_eq!(d.diff(127505, 1, &[1, 9]), vec![0, 0]);
        assert_eq!(d.diff(127505, 1, &[0, 4]), vec![0, -1]);
    }
}
