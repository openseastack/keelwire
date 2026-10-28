//! Line-oriented network gateways: Yacht Devices RAW (TCP/UDP) and Actisense
//! N2K ASCII (TCP, receive only).

use std::time::Duration;

use keelwire_core::formats::{parse_actisense_ascii, parse_yd_raw, yd_raw_tx};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineFormat {
    YdRaw,
    ActisenseAscii,
}

pub struct LineDriver {
    format: LineFormat,
    proto: String,
    addr: String,
    /// UDP: where to send TX datagrams (YD listens on the same port by default).
    udp_target: Option<String>,
}

impl LineDriver {
    pub fn yd(u: &Uri) -> Result<Self, InterfaceError> {
        let (proto, addr) = u.net(1457)?;
        Ok(LineDriver { format: LineFormat::YdRaw, proto, addr, udp_target: u.query.get("target").cloned() })
    }

    pub fn actisense_ascii(u: &Uri) -> Result<Self, InterfaceError> {
        let (proto, addr) = u.net(60001)?;
        if proto != "tcp" {
            return Err(InterfaceError::BadUri("actisense-ascii supports tcp:// only".into()));
        }
        Ok(LineDriver { format: LineFormat::ActisenseAscii, proto, addr, udp_target: None })
    }
}

/// Convert one received line into events.
pub fn line_event(format: LineFormat, line: &str) -> Option<BusEvent> {
    let now = clock::now_us();
    match format {
        LineFormat::YdRaw => {
            let (tx_echo, mut f) = parse_yd_raw(line)?;
            if tx_echo {
                return None; // our own transmissions echoed by the gateway
            }
            f.timestamp_us = now;
            Some(BusEvent::Frame(f))
        }
        LineFormat::ActisenseAscii => {
            let mut m = parse_actisense_ascii(line)?;
            m.timestamp_us = now;
            Some(BusEvent::Message(m))
        }
    }
}

impl CanInterface for LineDriver {
    fn name(&self) -> String {
        match self.format {
            LineFormat::YdRaw => format!("Yacht Devices RAW {}://{}", self.proto, self.addr),
            LineFormat::ActisenseAscii => format!("Actisense ASCII tcp://{}", self.addr),
        }
    }

    fn caps(&self) -> InterfaceCaps {
        match self.format {
            LineFormat::YdRaw => InterfaceCaps { raw_frames: true, can_transmit: true, ..Default::default() },
            LineFormat::ActisenseAscii => InterfaceCaps { raw_frames: false, can_transmit: false, ..Default::default() },
        }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            if self.proto == "udp" {
                run_udp(*self, events, outbound).await;
                return;
            }
            let mut backoff = Duration::from_millis(500);
            loop {
                let _ = events.send(BusEvent::Status(InterfaceStatus::Connecting)).await;
                match TcpStream::connect(&self.addr).await {
                    Ok(stream) => {
                        backoff = Duration::from_millis(500);
                        let _ = stream.set_nodelay(true);
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: self.addr.clone() })).await;
                        let (r, mut w) = stream.into_split();
                        let mut lines = BufReader::new(r).lines();
                        let reason = loop {
                            tokio::select! {
                                l = lines.next_line() => match l {
                                    Ok(Some(line)) => {
                                        if let Some(ev) = line_event(self.format, &line) {
                                            if events.send(ev).await.is_err() { return; }
                                        }
                                    }
                                    Ok(None) => break "connection closed".to_string(),
                                    Err(e) => break e.to_string(),
                                },
                                o = outbound.recv() => match o {
                                    Some(Outbound::Frame(f)) if self.format == LineFormat::YdRaw => {
                                        if let Err(e) = w.write_all(yd_raw_tx(&f).as_bytes()).await { break e.to_string(); }
                                    }
                                    Some(_) => {}
                                    None => return,
                                }
                            }
                        };
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason })).await;
                    }
                    Err(e) => {
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason: e.to_string() })).await;
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        })
    }
}

async fn run_udp(d: LineDriver, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) {
    let sock = match UdpSocket::bind(&d.addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason: format!("bind {}: {e}", d.addr) })).await;
            return;
        }
    };
    let _ = sock.set_broadcast(true);
    let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: format!("udp {}", d.addr) })).await;
    let mut buf = vec![0u8; 65536];
    let mut last_peer = None;
    loop {
        tokio::select! {
            r = sock.recv_from(&mut buf) => match r {
                Ok((n, peer)) => {
                    last_peer = Some(peer);
                    let text = String::from_utf8_lossy(&buf[..n]);
                    for line in text.lines() {
                        if let Some(ev) = line_event(d.format, line) {
                            if events.send(ev).await.is_err() { return; }
                        }
                    }
                }
                Err(e) => {
                    let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason: e.to_string() })).await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            },
            o = outbound.recv() => match o {
                Some(Outbound::Frame(f)) => {
                    let line = yd_raw_tx(&f);
                    let target = d.udp_target.clone().or_else(|| last_peer.map(|p| p.to_string()));
                    if let Some(t) = target {
                        let _ = sock.send_to(line.as_bytes(), t).await;
                    }
                }
                Some(_) => {}
                None => return,
            }
        }
    }
}
