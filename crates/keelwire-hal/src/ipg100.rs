//! Maretron IPG100 TCP client.
//!
//! Protocol as documented by the CANboat project (`maretron-ipg`, Apache-2.0):
//! * TCP port 6543 (bus 0) / 6553 (bus 1).
//! * Handshake: client sends `CONNECT\t"<password>"\t\tMOBILE\0`; server
//!   answers NUL-terminated text lines (`CONNECTED\t<serial>`, `NO`,
//!   `SERVER_VERSION\t…`); client then sends `SET_MODE\tBINARY\0`.
//! * Binary frames: `0xA5 · F1 · PF · PS · SA · LEN [· LEN_HI] · payload`,
//!   F1 = `0x80 | prio<<4 | msg_type<<1 | DP`; msg_type 3 uses a 16-bit LE
//!   length. Payloads are complete (reassembled) messages.
//! * TX: same framing, SA = 0xFF (the IPG substitutes its own address),
//!   msg_type 1 when len ≤ 8 else 2.

use std::time::Duration;

use keelwire_core::N2kMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub const DEFAULT_PORT: u16 = 6543;
const SYNC: u8 = 0xA5;

pub struct Ipg100 {
    addr: String,
    password: String,
}

impl Ipg100 {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        let (_, addr) = u.net(DEFAULT_PORT)?;
        Ok(Ipg100 { addr, password: u.query.get("password").cloned().unwrap_or_default() })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    NeedMore,
    Invalid,
    Frame { msg: N2kMessage, consumed: usize },
}

pub fn parse_frame(b: &[u8]) -> Parsed {
    if b.is_empty() {
        return Parsed::NeedMore;
    }
    if b[0] != SYNC {
        return Parsed::Invalid;
    }
    if b.len() < 6 {
        return Parsed::NeedMore;
    }
    let f1 = b[1];
    if f1 & 0x80 == 0 {
        return Parsed::Invalid;
    }
    let (pf, ps, sa) = (b[2], b[3], b[4]);
    let prio = (f1 >> 4) & 0x07;
    let msg_type = (f1 >> 1) & 0x03;
    let dp = (f1 & 0x01) as u32;
    let (start, len) = if msg_type == 3 {
        if b.len() < 7 {
            return Parsed::NeedMore;
        }
        (7, u16::from_le_bytes([b[5], b[6]]) as usize)
    } else {
        (6, b[5] as usize)
    };
    if b.len() < start + len {
        return Parsed::NeedMore;
    }
    let (pgn, dst) = if pf < 0xF0 { (dp << 16 | (pf as u32) << 8, ps) } else { (dp << 16 | (pf as u32) << 8 | ps as u32, 0xFF) };
    Parsed::Frame { msg: N2kMessage { timestamp_us: 0, priority: prio, pgn, source: sa, destination: dst, data: b[start..start + len].to_vec() }, consumed: start + len }
}

pub fn build_frame(m: &N2kMessage) -> Vec<u8> {
    let msg_type: u8 = if m.data.len() <= 8 { 1 } else { 2 };
    let dp = ((m.pgn >> 16) & 1) as u8;
    let pf = ((m.pgn >> 8) & 0xFF) as u8;
    let ps = if pf < 0xF0 { m.destination } else { (m.pgn & 0xFF) as u8 };
    let mut v = vec![SYNC, 0x80 | (m.priority & 7) << 4 | msg_type << 1 | dp, pf, ps, 0xFF, m.data.len().min(255) as u8];
    v.extend_from_slice(&m.data[..m.data.len().min(255)]);
    v
}

impl CanInterface for Ipg100 {
    fn name(&self) -> String {
        format!("Maretron IPG100 {}", self.addr)
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: false, can_transmit: true, tx_messages: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(500);
            loop {
                let _ = events.send(BusEvent::Status(InterfaceStatus::Connecting)).await;
                let reason = match TcpStream::connect(&self.addr).await {
                    Ok(mut s) => {
                        let _ = s.set_nodelay(true);
                        let hello = format!("CONNECT\t\"{}\"\t\tMOBILE\0", self.password);
                        if let Err(e) = s.write_all(hello.as_bytes()).await {
                            e.to_string()
                        } else {
                            backoff = Duration::from_millis(500);
                            match session(&mut s, &events, &mut outbound).await {
                                Some(r) => r,
                                None => return,
                            }
                        }
                    }
                    Err(e) => e.to_string(),
                };
                let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason })).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        })
    }
}

/// Runs one connection. Returns a disconnect reason, or None to stop for good.
async fn session(s: &mut TcpStream, events: &mpsc::Sender<BusEvent>, outbound: &mut mpsc::Receiver<Outbound>) -> Option<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    let mut streaming = false;
    loop {
        tokio::select! {
            r = s.read(&mut tmp) => {
                let n = match r { Ok(0) => return Some("connection closed".into()), Ok(n) => n, Err(e) => return Some(e.to_string()) };
                buf.extend_from_slice(&tmp[..n]);
                loop {
                    if buf.is_empty() { break; }
                    if !streaming || buf[0] & 0x80 == 0 && buf[0] != SYNC {
                        // NUL-terminated control / text message
                        let Some(nul) = buf.iter().position(|&c| c == 0) else { break };
                        let line = String::from_utf8_lossy(&buf[..nul]).to_string();
                        buf.drain(..=nul);
                        let (cmd, rest) = line.split_once('\t').unwrap_or((line.as_str(), ""));
                        match cmd {
                            "CONNECTED" => {
                                if let Err(e) = s.write_all(b"SET_MODE\tBINARY\0").await { return Some(e.to_string()); }
                                streaming = true;
                                let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: format!("IPG100 serial {rest}") })).await;
                            }
                            "NO" => return Some("authentication rejected by IPG100 (check ?password=)".into()),
                            _ => {}
                        }
                        continue;
                    }
                    if buf[0] == b'2' || buf[0] == b'3' {
                        // ASCII / video side channels: skip to NUL
                        let Some(nul) = buf.iter().position(|&c| c == 0) else { break };
                        buf.drain(..=nul);
                        continue;
                    }
                    match parse_frame(&buf) {
                        Parsed::NeedMore => break,
                        Parsed::Invalid => { buf.remove(0); }
                        Parsed::Frame { mut msg, consumed } => {
                            buf.drain(..consumed);
                            msg.timestamp_us = clock::now_us();
                            if events.send(BusEvent::Message(msg)).await.is_err() { return None; }
                        }
                    }
                }
            }
            o = outbound.recv() => match o {
                Some(Outbound::Message(m)) if streaming => {
                    if let Err(e) = s.write_all(&build_frame(&m)).await { return Some(e.to_string()); }
                }
                Some(_) => {}
                None => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let m = N2kMessage { timestamp_us: 0, priority: 3, pgn: 126208, source: 0xFF, destination: 0x23, data: (0..20).collect() };
        let b = build_frame(&m);
        match parse_frame(&b) {
            Parsed::Frame { msg, consumed } => {
                assert_eq!(consumed, b.len());
                assert_eq!(msg, m);
            }
            p => panic!("{p:?}"),
        }
        let m2 = N2kMessage { timestamp_us: 0, priority: 2, pgn: 127250, source: 0xFF, destination: 0xFF, data: vec![1; 8] };
        let b2 = build_frame(&m2);
        assert!(matches!(parse_frame(&b2[..5]), Parsed::NeedMore));
        assert!(matches!(parse_frame(&b2), Parsed::Frame { .. }));
    }
}
