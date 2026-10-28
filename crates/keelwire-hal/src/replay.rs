//! Replay recorded traffic with original timing (scaled by `speed`).

use std::path::PathBuf;
use std::time::Duration;

use keelwire_core::formats::{parse_any, LineItem};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub struct Replay {
    path: PathBuf,
    speed: f64,
    looped: bool,
}

impl Replay {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        if u.path.is_empty() {
            return Err(InterfaceError::BadUri("replay:<file>".into()));
        }
        Ok(Replay { path: PathBuf::from(&u.path), speed: u.q("speed").unwrap_or(1.0f64).max(0.0), looped: u.flag("loop") })
    }
}

/// Load a capture as a list of items with timestamps (µs, arbitrary origin).
pub fn load(path: &std::path::Path) -> Result<Vec<LineItem>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() >= 4 {
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        if matches!(magic, 0x0A0D_0D0A | 0xA1B2_C3D4 | 0xD4C3_B2A1 | 0xA1B2_3C4D | 0x4D3C_B2A1) {
            return keelwire_core::pcap::parse_bytes(&bytes).map(|v| v.into_iter().map(LineItem::Frame).collect());
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut out: Vec<LineItem> = text.lines().filter_map(parse_any).collect();
    if out.is_empty() {
        return Err("no recognizable frames in file".into());
    }
    // Lines without usable timestamps (e.g. plain candump) get 10 ms spacing.
    let all_zero = out.iter().all(|i| ts(i) == 0);
    if all_zero {
        for (n, i) in out.iter_mut().enumerate() {
            set_ts(i, n as u64 * 10_000);
        }
    }
    Ok(out)
}

fn ts(i: &LineItem) -> u64 {
    match i {
        LineItem::Frame(f) => f.timestamp_us,
        LineItem::Message(m) => m.timestamp_us,
    }
}

fn set_ts(i: &mut LineItem, t: u64) {
    match i {
        LineItem::Frame(f) => f.timestamp_us = t,
        LineItem::Message(m) => m.timestamp_us = t,
    }
}

impl CanInterface for Replay {
    fn name(&self) -> String {
        format!("Replay {}", self.path.display())
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: true, hw_timestamps: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let items = match load(&self.path) {
                Ok(v) => v,
                Err(e) => {
                    let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason: e })).await;
                    return;
                }
            };
            let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: format!("{} items from {}", items.len(), self.path.display()) })).await;
            loop {
                let first = ts(&items[0]);
                let base = clock::now_us();
                for it in &items {
                    // Drain (and drop) any outbound traffic: replays are read-only.
                    while outbound.try_recv().is_ok() {}
                    let rel = ts(it).saturating_sub(first);
                    let scaled = if self.speed > 0.0 { (rel as f64 / self.speed) as u64 } else { 0 };
                    let due = base + scaled;
                    let now = clock::now_us();
                    if due > now + 1000 {
                        tokio::time::sleep(Duration::from_micros(due - now)).await;
                    }
                    let mut it = it.clone();
                    set_ts(&mut it, base + scaled);
                    let ev = match it {
                        LineItem::Frame(f) => BusEvent::Frame(f),
                        LineItem::Message(m) => BusEvent::Message(m),
                    };
                    if events.send(ev).await.is_err() {
                        return;
                    }
                }
                if !self.looped {
                    break;
                }
            }
            let _ = events.send(BusEvent::Status(InterfaceStatus::Finished)).await;
        })
    }
}
