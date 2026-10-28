//! Hardware abstraction layer: every bus interface runs as a Tokio task that
//! emits [`BusEvent`]s and consumes [`Outbound`] traffic.
//!
//! Interfaces are selected with a URI (see [`open`]):
//!
//! | URI | Driver |
//! |---|---|
//! | `socketcan:can0` | Linux SocketCAN |
//! | `actisense:/dev/ttyUSB0?baud=115200` | Actisense NGT-1 (BST binary) |
//! | `actisense-ascii:tcp://host:port` | Actisense N2K ASCII over TCP (W2K-1, NGX-1 via network) |
//! | `ydraw:tcp://host:1457`, `ydraw:udp://0.0.0.0:1456` | Yacht Devices RAW |
//! | `ipg100:tcp://host[:6543]?password=` | Maretron IPG100 |
//! | `usb100:/dev/ttyUSB0` | Maretron USB100 (pending protocol extraction) |
//! | `signalk:ws://host:3000/plugins/keelwire-tap/stream` | Signal K tap plugin |
//! | `replay:/path/file?speed=1&loop=true` | pcap/pcapng/candump/canboat/YD logs |
//! | `sim:` / `sim:?conflict=1` | built-in simulator |
//! | `loopback:` | echo (tests) |

use std::collections::HashMap;

use keelwire_core::{N2kMessage, RawCanFrame};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub mod actisense;
pub mod clock;
pub mod ipg100;
pub mod lines;
pub mod loopback;
pub mod replay;
pub mod signalk;
pub mod serial;
pub mod sim;
#[cfg(target_os = "linux")]
pub mod socketcan;
pub mod usb100;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BusErrorKind {
    BusOff,
    ErrorPassive,
    Warning,
    Overrun,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InterfaceStatus {
    Connecting,
    Connected { detail: String },
    Disconnected { reason: String },
    Finished,
}

#[derive(Clone, Debug)]
pub enum BusEvent {
    Frame(RawCanFrame),
    Message(N2kMessage),
    ErrorFrame { timestamp_us: u64, kind: BusErrorKind },
    Status(InterfaceStatus),
}

#[derive(Clone, Debug)]
pub enum Outbound {
    Frame(RawCanFrame),
    Message(N2kMessage),
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct InterfaceCaps {
    /// Delivers individual CAN frames (false: only reassembled messages).
    pub raw_frames: bool,
    pub can_transmit: bool,
    /// Accepts whole messages on TX (engine does not fragment).
    pub tx_messages: bool,
    pub error_frames: bool,
    pub hw_timestamps: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InterfaceError {
    #[error("invalid interface URI: {0}")]
    BadUri(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// A bus interface. `spawn` consumes the driver and runs it until the
/// outbound channel closes, the task is aborted, or (for replays) input ends.
pub trait CanInterface: Send {
    fn name(&self) -> String;
    fn caps(&self) -> InterfaceCaps;
    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()>;
}

/// Running interface.
pub struct InterfaceHandle {
    pub uri: String,
    pub name: String,
    pub caps: InterfaceCaps,
    pub tx: mpsc::Sender<Outbound>,
    join: JoinHandle<()>,
}

impl InterfaceHandle {
    pub fn stop(&self) {
        self.join.abort();
    }
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }
}

impl Drop for InterfaceHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
}

/// Parsed `scheme:rest?k=v&k2=v2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uri {
    pub scheme: String,
    pub path: String,
    pub query: HashMap<String, String>,
}

impl Uri {
    pub fn parse(s: &str) -> Result<Self, InterfaceError> {
        let (scheme, rest) = s.split_once(':').ok_or_else(|| InterfaceError::BadUri(s.into()))?;
        let (path, q) = rest.split_once('?').unwrap_or((rest, ""));
        let query = q
            .split('&')
            .filter(|x| !x.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (k.to_string(), percent_decode(v))
            })
            .collect();
        Ok(Uri { scheme: scheme.to_ascii_lowercase(), path: path.to_string(), query })
    }

    pub fn q<T: std::str::FromStr>(&self, k: &str) -> Option<T> {
        self.query.get(k).and_then(|v| v.parse().ok())
    }

    pub fn flag(&self, k: &str) -> bool {
        matches!(self.query.get(k).map(|s| s.as_str()), Some("1" | "true" | "yes" | ""))
    }

    /// For `tcp://host:port` or `udp://host:port` paths: (proto, host:port).
    pub fn net(&self, default_port: u16) -> Result<(String, String), InterfaceError> {
        let (proto, hp) = self.path.split_once("://").unwrap_or(("tcp", self.path.as_str()));
        let hp = hp.trim_end_matches('/');
        let hp = if hp.rsplit_once(':').map(|(_, p)| p.parse::<u16>().is_ok()).unwrap_or(false) { hp.to_string() } else { format!("{hp}:{default_port}") };
        Ok((proto.to_ascii_lowercase(), hp))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Construct a driver from a URI.
pub fn create(uri: &str) -> Result<Box<dyn CanInterface>, InterfaceError> {
    let u = Uri::parse(uri)?;
    Ok(match u.scheme.as_str() {
        "loopback" => Box::new(loopback::Loopback),
        "sim" => Box::new(sim::Simulator::new(&u)),
        "replay" => Box::new(replay::Replay::new(&u)?),
        "ydraw" => Box::new(lines::LineDriver::yd(&u)?),
        "actisense-ascii" => Box::new(lines::LineDriver::actisense_ascii(&u)?),
        "actisense" | "ngt1" if !serial::AVAILABLE => return Err(InterfaceError::Unsupported("USB gateways cannot be used on phones and tablets; use a Wi-Fi gateway".into())),
        "actisense" | "ngt1" => Box::new(actisense::Actisense::new(&u)?),
        "ipg100" => Box::new(ipg100::Ipg100::new(&u)?),
        "usb100" => Box::new(usb100::Usb100::new(&u)?),
        "signalk" => Box::new(signalk::SignalK::new(&u)?),
        #[cfg(target_os = "linux")]
        "socketcan" | "can" => Box::new(socketcan::SocketCan::new(&u)?),
        #[cfg(not(target_os = "linux"))]
        "socketcan" | "can" => return Err(InterfaceError::Unsupported("SocketCAN is only available on Linux".into())),
        other => return Err(InterfaceError::BadUri(format!("unknown interface type '{other}'"))),
    })
}

/// Create and start an interface.
pub fn open(uri: &str, events: mpsc::Sender<BusEvent>) -> Result<InterfaceHandle, InterfaceError> {
    let drv = create(uri)?;
    let name = drv.name();
    let caps = drv.caps();
    let (tx, rx) = mpsc::channel(1024);
    let join = drv.spawn(events, rx);
    Ok(InterfaceHandle { uri: uri.to_string(), name, caps, tx, join })
}

/// Interface kinds for the Settings page.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InterfaceKind {
    pub scheme: &'static str,
    pub label: &'static str,
    pub example: &'static str,
    pub available: bool,
    pub note: &'static str,
}

/// A serial port that may be an NMEA 2000 gateway (USB gateways appear as
/// serial ports), with a suggested interface URI when it is recognised.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SerialPortInfo {
    pub path: String,
    pub description: String,
    pub suggested: Option<String>,
}

#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn serial_ports() -> Vec<SerialPortInfo> {
    Vec::new()
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn serial_ports() -> Vec<SerialPortInfo> {
    let mut out = Vec::new();
    for p in serial::ports() {
        let (desc, text) = match &p.port_type {
            tokio_serial::SerialPortType::UsbPort(u) => {
                let d = [u.manufacturer.clone(), u.product.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
                let d = if d.is_empty() { format!("USB {:04x}:{:04x}", u.vid, u.pid) } else { d };
                (d.clone(), format!("{d} {}", u.serial_number.clone().unwrap_or_default()).to_lowercase())
            }
            tokio_serial::SerialPortType::BluetoothPort => ("Bluetooth".into(), String::new()),
            tokio_serial::SerialPortType::PciPort => ("Built-in serial port".into(), String::new()),
            tokio_serial::SerialPortType::Unknown => ("Serial port".into(), String::new()),
        };
        let suggested = if text.contains("ngt") || text.contains("actisense") {
            Some(format!("actisense:{}", p.port_name))
        } else if text.contains("usb100") || text.contains("maretron") {
            Some(format!("usb100:{}", p.port_name))
        } else {
            None
        };
        // Skip the many virtual tty/cu duplicates on macOS and Linux consoles.
        if p.port_name.starts_with("/dev/tty.") || p.port_name.starts_with("/dev/ttyS") && matches!(p.port_type, tokio_serial::SerialPortType::Unknown) {
            continue;
        }
        out.push(SerialPortInfo { path: p.port_name, description: desc, suggested });
    }
    out
}

pub fn kinds() -> Vec<InterfaceKind> {
    // Phones and tablets have no serial ports; they reach the bus over Wi-Fi.
    let serial = serial::AVAILABLE;
    vec![
        InterfaceKind { scheme: "sim", label: "Simulator", example: "sim:", available: true, note: "Synthetic vessel; add ?conflict=1 to inject an instance conflict" },
        InterfaceKind { scheme: "ipg100", label: "Maretron IPG100", example: "ipg100:tcp://192.168.1.100:6543", available: true, note: "Port 6543 = bus 0, 6553 = bus 1; ?password=" },
        InterfaceKind { scheme: "usb100", label: "Maretron USB100", example: "usb100:/dev/ttyUSB0", available: false, note: "Protocol extraction pending (spec Appendix A)" },
        InterfaceKind { scheme: "socketcan", label: "SocketCAN", example: "socketcan:can0", available: cfg!(target_os = "linux"), note: "Linux only; set bitrate 250000" },
        InterfaceKind { scheme: "actisense", label: "Actisense NGT-1", example: "actisense:/dev/ttyUSB0?baud=115200", available: serial, note: "Delivers reassembled messages" },
        InterfaceKind { scheme: "ydraw", label: "Yacht Devices RAW", example: "ydraw:tcp://192.168.4.1:1457", available: true, note: "TCP or udp://0.0.0.0:1456" },
        InterfaceKind { scheme: "actisense-ascii", label: "Actisense N2K ASCII (TCP)", example: "actisense-ascii:tcp://host:60001", available: true, note: "" },
        InterfaceKind { scheme: "signalk", label: "Signal K (keelwire-tap plugin)", example: "signalk:ws://localhost:3000/plugins/signalk-keelwire-tap/stream", available: true, note: "Requires tools/signalk-keelwire-tap" },
        InterfaceKind { scheme: "replay", label: "Replay file", example: "replay:/path/capture.pcapng?speed=1&loop=true", available: true, note: "pcap, pcapng, candump, canboat, YD RAW, Actisense ASCII" },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_parsing() {
        let u = Uri::parse("ipg100:tcp://10.0.0.5?password=a%20b").unwrap();
        assert_eq!(u.scheme, "ipg100");
        assert_eq!(u.query["password"], "a b");
        assert_eq!(u.net(6543).unwrap(), ("tcp".into(), "10.0.0.5:6543".into()));
        let u = Uri::parse("ydraw:udp://0.0.0.0:1456").unwrap();
        assert_eq!(u.net(1457).unwrap(), ("udp".into(), "0.0.0.0:1456".into()));
        let u = Uri::parse("replay:/tmp/x.log?speed=2&loop").unwrap();
        assert_eq!(u.q::<f64>("speed"), Some(2.0));
        assert!(u.flag("loop"));
    }
}
