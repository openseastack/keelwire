//! ISO 11783-3 / SAE J1939-21 Transport Protocol (TP.CM 60416, TP.DT 60160).
//!
//! Handles BAM (broadcast) and RTS/CTS (connection mode) sessions, both
//! passively (sniffing sessions between other nodes) and actively (answering
//! CTS when we are the destination).

use std::collections::HashMap;

pub const PGN_TP_CM: u32 = 60416;
pub const PGN_TP_DT: u32 = 60160;
pub const TP_MAX: usize = 1785;

pub const CM_RTS: u8 = 16;
pub const CM_CTS: u8 = 17;
pub const CM_EOMA: u8 = 19;
pub const CM_BAM: u8 = 32;
pub const CM_ABORT: u8 = 255;

pub const T1_US: u64 = 750_000;
pub const T2_US: u64 = 1_250_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpMessage {
    pub pgn: u32,
    pub source: u8,
    pub destination: u8,
    pub priority: u8,
    pub data: Vec<u8>,
}

/// Frames the caller must transmit in response (only when we are the receiver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpReply {
    pub pgn: u32,
    pub destination: u8,
    pub data: [u8; 8],
}

#[derive(Debug)]
struct Session {
    pgn: u32,
    size: usize,
    packets: u8,
    data: Vec<u8>,
    received: Vec<bool>,
    last_us: u64,
    priority: u8,
    bam: bool,
}

#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TpStats {
    pub completed: u64,
    pub aborted: u64,
    pub timeouts: u64,
}

#[derive(Default)]
pub struct TpTracker {
    sessions: HashMap<(u8, u8), Session>, // (source, destination)
    /// Our own address; RTS addressed to it are answered with CTS.
    pub own_address: Option<u8>,
    pub stats: TpStats,
}


fn pgn_from(b: &[u8]) -> u32 {
    b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16
}

impl TpTracker {
    /// Process a TP.CM or TP.DT frame. Returns a completed message and/or
    /// frames to transmit.
    pub fn push(&mut self, ts_us: u64, priority: u8, pgn: u32, source: u8, destination: u8, d: &[u8]) -> (Option<TpMessage>, Vec<TpReply>) {
        let mut replies = Vec::new();
        if d.len() < 8 {
            return (None, replies);
        }
        match pgn {
            PGN_TP_CM => {
                let ctrl = d[0];
                match ctrl {
                    CM_BAM | CM_RTS => {
                        let size = (d[1] as usize) | (d[2] as usize) << 8;
                        let packets = d[3];
                        let target = pgn_from(&d[5..8]);
                        if size == 0 || size > TP_MAX || packets == 0 {
                            return (None, replies);
                        }
                        let bam = ctrl == CM_BAM;
                        self.sessions.insert(
                            (source, destination),
                            Session {
                                pgn: target,
                                size,
                                packets,
                                data: vec![0xFF; packets as usize * 7],
                                received: vec![false; packets as usize],
                                last_us: ts_us,
                                priority,
                                bam,
                            },
                        );
                        if !bam && Some(destination) == self.own_address {
                            // Clear to send everything, starting at packet 1.
                            let mut cts = [0xFFu8; 8];
                            cts[0] = CM_CTS;
                            cts[1] = packets;
                            cts[2] = 1;
                            cts[5..8].copy_from_slice(&d[5..8]);
                            replies.push(TpReply { pgn: PGN_TP_CM, destination: source, data: cts });
                        }
                    }
                    CM_ABORT
                        // Abort can come from either side.
                        if (self.sessions.remove(&(source, destination)).is_some()
                            || self.sessions.remove(&(destination, source)).is_some())
                        => {
                            self.stats.aborted += 1;
                        }
                    _ => {}
                }
                (None, replies)
            }
            PGN_TP_DT => {
                let key = (source, destination);
                let Some(s) = self.sessions.get_mut(&key) else { return (None, replies) };
                let n = d[0];
                if n == 0 || n > s.packets {
                    return (None, replies);
                }
                let idx = (n - 1) as usize;
                s.data[idx * 7..idx * 7 + 7].copy_from_slice(&d[1..8]);
                s.received[idx] = true;
                s.last_us = ts_us;
                if s.received.iter().all(|x| *x) {
                    let s = self.sessions.remove(&key).unwrap();
                    self.stats.completed += 1;
                    if !s.bam && Some(destination) == self.own_address {
                        let mut ack = [0xFFu8; 8];
                        ack[0] = CM_EOMA;
                        ack[1] = (s.size & 0xFF) as u8;
                        ack[2] = (s.size >> 8) as u8;
                        ack[3] = s.packets;
                        ack[5] = (s.pgn & 0xFF) as u8;
                        ack[6] = ((s.pgn >> 8) & 0xFF) as u8;
                        ack[7] = ((s.pgn >> 16) & 0xFF) as u8;
                        replies.push(TpReply { pgn: PGN_TP_CM, destination: source, data: ack });
                    }
                    let mut data = s.data;
                    data.truncate(s.size);
                    return (Some(TpMessage { pgn: s.pgn, source, destination, priority: s.priority, data }), replies);
                }
                (None, replies)
            }
            _ => (None, replies),
        }
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let before = self.sessions.len();
        self.sessions.retain(|_, s| now_us.saturating_sub(s.last_us) <= T2_US);
        let n = before - self.sessions.len();
        self.stats.timeouts += n as u64;
        n
    }
}

/// Build a BAM transfer: returns (pgn, data) frames — first TP.CM then TP.DT.
pub fn bam_frames(pgn: u32, data: &[u8]) -> Vec<(u32, [u8; 8])> {
    assert!(data.len() > 8 && data.len() <= TP_MAX);
    let packets = data.len().div_ceil(7);
    let mut out = Vec::with_capacity(packets + 1);
    let mut cm = [0xFFu8; 8];
    cm[0] = CM_BAM;
    cm[1] = (data.len() & 0xFF) as u8;
    cm[2] = (data.len() >> 8) as u8;
    cm[3] = packets as u8;
    cm[5] = (pgn & 0xFF) as u8;
    cm[6] = ((pgn >> 8) & 0xFF) as u8;
    cm[7] = ((pgn >> 16) & 0xFF) as u8;
    out.push((PGN_TP_CM, cm));
    for i in 0..packets {
        let mut dt = [0xFFu8; 8];
        dt[0] = (i + 1) as u8;
        let start = i * 7;
        let end = (start + 7).min(data.len());
        dt[1..1 + end - start].copy_from_slice(&data[start..end]);
        out.push((PGN_TP_DT, dt));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bam_roundtrip() {
        let payload: Vec<u8> = (0..9u8).collect();
        let frames = bam_frames(65240, &payload);
        assert_eq!(frames.len(), 3);
        let mut t = TpTracker::default();
        let mut got = None;
        for (pgn, f) in frames {
            let (m, r) = t.push(0, 7, pgn, 0x80, 255, &f);
            assert!(r.is_empty());
            if m.is_some() {
                got = m;
            }
        }
        let m = got.unwrap();
        assert_eq!(m.pgn, 65240);
        assert_eq!(m.data, payload);
    }

    #[test]
    fn rts_cts_to_us() {
        let payload: Vec<u8> = (0..20u8).collect();
        let mut t = TpTracker { own_address: Some(0x80), ..Default::default() };
        let mut rts = [0xFFu8; 8];
        rts[0] = CM_RTS;
        rts[1] = 20;
        rts[2] = 0;
        rts[3] = 3;
        rts[5..8].copy_from_slice(&[0x10, 0xF0, 0x01]);
        let (_, replies) = t.push(0, 7, PGN_TP_CM, 0x23, 0x80, &rts);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].data[0], CM_CTS);
        let mut done = None;
        for i in 0..3 {
            let mut dt = [0xFFu8; 8];
            dt[0] = i + 1;
            for j in 0..7 {
                let k = i as usize * 7 + j;
                if k < 20 {
                    dt[1 + j] = payload[k];
                }
            }
            let (m, r) = t.push(0, 7, PGN_TP_DT, 0x23, 0x80, &dt);
            if m.is_some() {
                assert_eq!(r[0].data[0], CM_EOMA);
                done = m;
            }
        }
        let m = done.unwrap();
        assert_eq!(m.pgn, 0x01F010);
        assert_eq!(m.data, payload);
    }
}
