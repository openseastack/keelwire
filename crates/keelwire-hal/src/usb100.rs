//! Maretron USB100 gateway.
//!
//! The USB100 wire protocol is not publicly documented. Until it is (from
//! community documentation or captures on owned hardware), this driver reports a clear
//! "unsupported" status instead of guessing at a protocol, which could put
//! unintended traffic on a live vessel bus.

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{BusEvent, CanInterface, InterfaceCaps, InterfaceError, InterfaceStatus, Outbound, Uri};

pub struct Usb100 {
    port: String,
}

impl Usb100 {
    pub fn new(u: &Uri) -> Result<Self, InterfaceError> {
        Ok(Usb100 { port: u.path.clone() })
    }
}

impl CanInterface for Usb100 {
    fn name(&self) -> String {
        format!("Maretron USB100 {}", self.port)
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps::default()
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, _outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let _ = events
                .send(BusEvent::Status(InterfaceStatus::Disconnected {
                    reason: "USB100 protocol support is pending protocol extraction (spec Appendix A). Use an IPG100, SocketCAN, Actisense or Yacht Devices gateway meanwhile.".into(),
                }))
                .await;
        })
    }
}
