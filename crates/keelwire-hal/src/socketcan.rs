//! Linux SocketCAN driver (`socketcan:can0`, `socketcan:vcan0`).

use std::time::Duration;

use keelwire_core::RawCanFrame;
use socketcan::tokio::CanSocket;
use socketcan::{CanFrame, EmbeddedFrame, ExtendedId, Frame, SocketOptions};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusErrorKind, BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub struct SocketCan {
    iface: String,
}

impl SocketCan {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        let iface = u.path.trim_start_matches("//").to_string();
        if iface.is_empty() {
            return Err(InterfaceError::BadUri("socketcan:<interface>".into()));
        }
        Ok(SocketCan { iface })
    }
}

impl CanInterface for SocketCan {
    fn name(&self) -> String {
        format!("SocketCAN {}", self.iface)
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: true, can_transmit: true, error_frames: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(500);
            loop {
                let _ = events.send(BusEvent::Status(InterfaceStatus::Connecting)).await;
                let reason = match CanSocket::open(&self.iface) {
                    Ok(sock) => {
                        let _ = sock.set_error_filter_accept_all();
                        backoff = Duration::from_millis(500);
                        let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: self.iface.clone() })).await;
                        loop {
                            tokio::select! {
                                r = sock.read_frame() => match r {
                                    Ok(CanFrame::Data(f)) => {
                                        let ev = BusEvent::Frame(RawCanFrame::new(clock::now_us(), f.raw_id(), f.data()));
                                        if !f.is_extended() { continue; } // N2K uses 29-bit ids only
                                        if events.send(ev).await.is_err() { return; }
                                    }
                                    Ok(CanFrame::Error(e)) => {
                                        let kind = if matches!(e.into_error(), socketcan::CanError::BusOff) { BusErrorKind::BusOff } else { BusErrorKind::Other };
                                        let _ = events.send(BusEvent::ErrorFrame { timestamp_us: clock::now_us(), kind }).await;
                                    }
                                    Ok(_) => {}
                                    Err(e) => break e.to_string(),
                                },
                                o = outbound.recv() => match o {
                                    Some(Outbound::Frame(f)) => {
                                        let Some(id) = ExtendedId::new(f.can_id & 0x1FFF_FFFF) else { continue };
                                        let Some(frame) = CanFrame::new(id, f.payload()) else { continue };
                                        if let Err(e) = sock.write_frame(frame).await { break e.to_string(); }
                                    }
                                    Some(Outbound::Message(_)) => {}
                                    None => return,
                                }
                            }
                        }
                    }
                    Err(e) => format!("{}: {e}", self.iface),
                };
                let _ = events.send(BusEvent::Status(InterfaceStatus::Disconnected { reason })).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        })
    }
}
