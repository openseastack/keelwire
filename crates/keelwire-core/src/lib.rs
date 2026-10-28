//! Keelwire core: pure NMEA 2000 protocol logic with no I/O.
//!
//! Enable the `serde` feature (on by default) for JSON (de)serialization,
//! the CANboat database loader and proprietary schema files.

pub mod analysis;
pub mod bits;
pub mod canboat;
pub mod fastpacket;
pub mod filter;
pub mod formats;
pub mod frame;
pub mod groupfunction;
pub mod iso;
pub mod isotp;
pub mod metrics;
pub mod pcap;
pub mod proprietary;
pub mod registry;
pub mod sniffer;
pub mod traffic;
pub mod units;
pub mod value;

pub use frame::{CanId, N2kMessage, RawCanFrame};
pub use value::{Field, Value};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
