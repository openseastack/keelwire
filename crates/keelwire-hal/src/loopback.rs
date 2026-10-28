//! Loopback interface: transmitted frames are received back. Used in tests.

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceStatus, Outbound};

pub struct Loopback;

impl CanInterface for Loopback {
    fn name(&self) -> String {
        "Loopback".into()
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: true, can_transmit: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: "loopback".into() })).await;
            while let Some(o) = outbound.recv().await {
                let ev = match o {
                    Outbound::Frame(mut f) => {
                        f.timestamp_us = clock::now_us();
                        BusEvent::Frame(f)
                    }
                    Outbound::Message(mut m) => {
                        m.timestamp_us = clock::now_us();
                        BusEvent::Message(m)
                    }
                };
                if events.send(ev).await.is_err() {
                    break;
                }
            }
        })
    }
}
