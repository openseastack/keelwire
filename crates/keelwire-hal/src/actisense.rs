//! Actisense NGT-1 BST binary protocol over a serial port.
//!
//! Framing `DLE STX cmd len data… chk DLE ETX`, DLE (0x10) doubled inside;
//! checksum makes `cmd + len + Σdata + chk ≡ 0 (mod 256)`.
//! RX 0x93: prio · PGN(24 LE) · dst · src · timestamp(32 LE, ms) · len · data.
//! TX 0x94: prio · PGN(24 LE) · dst · len · data.
//! At start-up the driver sends BEM "Set Operating Mode = Rx All" (0xA1:
//! 11 02 00) so the gateway forwards every PGN (see CANboat actisense-serial).

use std::time::Duration;

use keelwire_core::N2kMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub const DLE: u8 = 0x10;
pub const STX: u8 = 0x02;
pub const ETX: u8 = 0x03;
pub const N2K_MSG_RECEIVED: u8 = 0x93;
pub const N2K_MSG_SEND: u8 = 0x94;
pub const NGT_MSG_RECEIVED: u8 = 0xA0;
pub const NGT_MSG_SEND: u8 = 0xA1;
pub const SET_OPERATING_MODE_RX_ALL: [u8; 3] = [0x11, 0x02, 0x00];

pub struct Actisense {
    port: String,
    baud: u32,
}

impl Actisense {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        if u.path.is_empty() {
            return Err(InterfaceError::BadUri("actisense:<serial port>".into()));
        }
        Ok(Actisense { port: u.path.clone(), baud: u.q("baud").unwrap_or(115_200) })
    }
}

/// Encode one BST packet.
pub fn encode(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![DLE, STX];
    let mut sum: u8 = cmd;
    let push = |v: &mut Vec<u8>, b: u8| {
        if b == DLE {
            v.push(DLE);
        }
        v.push(b);
    };
    push(&mut v, cmd);
    push(&mut v, payload.len() as u8);
    sum = sum.wrapping_add(payload.len() as u8);
    for &b in payload {
        push(&mut v, b);
        sum = sum.wrapping_add(b);
    }
    push(&mut v, 0u8.wrapping_sub(sum));
    v.push(DLE);
    v.push(ETX);
    v
}

/// Incremental BST decoder.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    in_msg: bool,
    esc: bool,
    pub bad_checksums: u64,
}

impl Decoder {
    /// Feed bytes; returns complete (cmd, payload) packets with valid checksum.
    pub fn feed(&mut self, data: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        for &c in data {
            if self.esc {
                self.esc = false;
                match c {
                    STX => {
                        self.in_msg = true;
                        self.buf.clear();
                    }
                    ETX if self.in_msg => {
                        self.in_msg = false;
                        if let Some(p) = self.finish() {
                            out.push(p);
                        }
                    }
                    DLE if self.in_msg => self.buf.push(DLE),
                    _ => self.in_msg = false,
                }
            } else if c == DLE {
                self.esc = true;
            } else if self.in_msg {
                self.buf.push(c);
                if self.buf.len() > 600 {
                    self.in_msg = false;
                }
            }
        }
        out
    }

    fn finish(&mut self) -> Option<(u8, Vec<u8>)> {
        let b = &self.buf;
        if b.len() < 3 {
            return None;
        }
        let sum = b.iter().fold(0u8, |a, x| a.wrapping_add(*x));
        if sum != 0 {
            self.bad_checksums += 1;
            return None;
        }
        let cmd = b[0];
        let len = b[1] as usize;
        let payload = b.get(2..2 + len)?.to_vec();
        Some((cmd, payload))
    }
}

/// Parse a 0x93 payload into a message.
pub fn parse_n2k_received(p: &[u8]) -> Option<N2kMessage> {
    if p.len() < 11 {
        return None;
    }
    let pgn = p[1] as u32 | (p[2] as u32) << 8 | (p[3] as u32) << 16;
    let len = p[10] as usize;
    let data = p.get(11..11 + len.min(p.len() - 11))?.to_vec();
    Some(N2kMessage { timestamp_us: 0, priority: p[0], pgn, source: p[5], destination: p[4], data })
}

pub fn n2k_send_payload(m: &N2kMessage) -> Vec<u8> {
    let mut v = vec![m.priority, (m.pgn & 0xFF) as u8, ((m.pgn >> 8) & 0xFF) as u8, ((m.pgn >> 16) & 0xFF) as u8, m.destination, m.data.len() as u8];
    v.extend_from_slice(&m.data);
    v
}

impl CanInterface for Actisense {
    fn name(&self) -> String {
        format!("Actisense NGT-1 {}", self.port)
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: false, can_transmit: true, tx_messages: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(500);
            loop {
                let _ = events.send(BusEvent::Status(InterfaceStatus::Connecting)).await;
                let reason = match crate::serial::open(&self.port, self.baud) {
                    Ok(mut port) => {
                        backoff = Duration::from_millis(500);
                        let _ = port.write_all(&encode(NGT_MSG_SEND, &SET_OPERATING_MODE_RX_ALL)).await;
                        // The NGT-1 drops writes for ~450 ms after a mode change.
                        tokio::time::sleep(Duration::from_millis(600)).await;
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: format!("{} @ {}", self.port, self.baud) })).await;
                        let mut dec = Decoder::default();
                        let mut buf = [0u8; 2048];
                        loop {
                            tokio::select! {
                                r = port.read(&mut buf) => match r {
                                    Ok(0) => break "port closed".to_string(),
                                    Ok(n) => {
                                        for (cmd, p) in dec.feed(&buf[..n]) {
                                            if cmd == N2K_MSG_RECEIVED {
                                                if let Some(mut m) = parse_n2k_received(&p) {
                                                    m.timestamp_us = clock::now_us();
                                                    if events.send(BusEvent::Message(m)).await.is_err() { return; }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => break e.to_string(),
                                },
                                o = outbound.recv() => match o {
                                    Some(Outbound::Message(m)) => {
                                        if let Err(e) = port.write_all(&encode(N2K_MSG_SEND, &n2k_send_payload(&m))).await { break e.to_string(); }
                                    }
                                    Some(Outbound::Frame(_)) => {}
                                    None => return,
                                }
                            }
                        }
                    }
                    Err(e) => e,
                };
                let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason })).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bst_roundtrip_with_stuffing() {
        // payload containing DLE bytes and a checksum that may be DLE
        let mut rx = vec![2u8, 0x10, 0xF1, 0x01, 0xFF, 0x10, 1, 2, 3, 4, 3];
        rx.extend_from_slice(&[0x10, 0xAA, 0x10]);
        let pkt = encode(N2K_MSG_RECEIVED, &rx);
        let mut d = Decoder::default();
        // feed in two chunks with leading garbage
        let mut stream = vec![0x55, 0x10, 0x03];
        stream.extend_from_slice(&pkt);
        let (a, b) = stream.split_at(7);
        let mut got = d.feed(a);
        got.extend(d.feed(b));
        assert_eq!(got, vec![(N2K_MSG_RECEIVED, rx.clone())]);
        let m = parse_n2k_received(&rx).unwrap();
        assert_eq!(m.pgn, 0x01F110);
        assert_eq!(m.source, 0x10);
        assert_eq!(m.data, vec![0x10, 0xAA, 0x10]);
    }

    #[test]
    fn checksum_rejects() {
        let mut pkt = encode(NGT_MSG_SEND, &SET_OPERATING_MODE_RX_ALL);
        let i = pkt.len() - 3;
        pkt[i] ^= 1;
        let mut d = Decoder::default();
        assert!(d.feed(&pkt).is_empty());
        assert_eq!(d.bad_checksums, 1);
    }
}
