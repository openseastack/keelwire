//! Serial ports (USB gateways such as the Actisense NGT-1).
//!
//! Desktop and Raspberry Pi only: iPhone, iPad and Android have no serial
//! ports, and the serial library links Apple's IOKit, which iOS apps may not
//! use, so it is not compiled there.

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod imp {
    use tokio_serial::SerialPortBuilderExt;

    pub type Port = tokio_serial::SerialStream;

    pub fn open(path: &str, baud: u32) -> Result<Port, String> {
        tokio_serial::new(path, baud).open_native_async().map_err(|e| e.to_string())
    }

    pub fn ports() -> Vec<tokio_serial::SerialPortInfo> {
        tokio_serial::available_ports().unwrap_or_default()
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod imp {
    pub type Port = tokio::io::DuplexStream;

    pub fn open(_path: &str, _baud: u32) -> Result<Port, String> {
        Err("serial ports are not available on phones and tablets; use a Wi-Fi gateway".into())
    }
}

pub use imp::*;

/// Serial ports exist on this platform.
pub const AVAILABLE: bool = !cfg!(any(target_os = "ios", target_os = "android"));
