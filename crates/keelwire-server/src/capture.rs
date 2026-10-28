//! Background capture files (pcapng, candump, canboat plain).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use keelwire_core::formats::{candump_line, canboat_plain_line, format_iso_time};
use keelwire_core::pcap::PcapngWriter;
use keelwire_core::{N2kMessage, RawCanFrame};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFormat {
    #[default]
    Pcapng,
    Candump,
    Canboat,
}

impl CaptureFormat {
    fn ext(&self) -> &'static str {
        match self {
            CaptureFormat::Pcapng => "pcapng",
            CaptureFormat::Candump => "log",
            CaptureFormat::Canboat => "csv",
        }
    }
}

enum Sink {
    Pcapng(PcapngWriter<BufWriter<File>>),
    Text(BufWriter<File>),
}

pub struct Capture {
    path: PathBuf,
    format: CaptureFormat,
    sink: Sink,
    frames: u64,
    messages: u64,
    bytes: u64,
    started: Instant,
    last_flush: Instant,
    iface: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureInfo {
    pub file: String,
    pub format: CaptureFormat,
    pub frames: u64,
    pub messages: u64,
    pub bytes: u64,
    pub seconds: u64,
}

pub fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect::<String>().trim_matches('.').to_string()
}

impl Capture {
    pub fn start(dir: &Path, format: CaptureFormat, name: Option<String>, iface: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let stamp = format_iso_time(keelwire_hal::clock::unix_now_us()).replace([':', '-'], "").replace('T', "-");
        let base = name.map(|n| sanitize(&n)).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("capture-{}", &stamp[..15.min(stamp.len())]));
        let path = dir.join(format!("{}.{}", base.trim_end_matches(&format!(".{}", format.ext())), format.ext()));
        let f = BufWriter::with_capacity(1 << 16, File::create(&path)?);
        let sink = match format {
            CaptureFormat::Pcapng => Sink::Pcapng(PcapngWriter::new(f, &format!("Keelwire {}", crate::VERSION), iface)?),
            _ => Sink::Text(f),
        };
        Ok(Capture { path, format, sink, frames: 0, messages: 0, bytes: 0, started: Instant::now(), last_flush: Instant::now(), iface: iface.to_string() })
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    pub fn write_frame(&mut self, unix_us: u64, f: &RawCanFrame) {
        self.frames += 1;
        match &mut self.sink {
            Sink::Pcapng(w) => {
                if w.write(unix_us, f).is_ok() {
                    self.bytes += 48;
                }
            }
            Sink::Text(w) if self.format == CaptureFormat::Candump => {
                let l = candump_line(unix_us, &self.iface.replace(' ', "_"), f);
                self.bytes += l.len() as u64;
                let _ = w.write_all(l.as_bytes());
            }
            _ => {}
        }
    }

    pub fn write_message(&mut self, unix_us: u64, m: &N2kMessage) {
        self.messages += 1;
        if let (Sink::Text(w), CaptureFormat::Canboat) = (&mut self.sink, self.format) {
            let l = canboat_plain_line(unix_us, m);
            self.bytes += l.len() as u64;
            let _ = w.write_all(l.as_bytes());
        }
    }

    pub fn maybe_flush(&mut self) {
        if self.last_flush.elapsed().as_secs() >= 2 {
            self.last_flush = Instant::now();
            match &mut self.sink {
                Sink::Pcapng(w) => {
                    let _ = w.flush();
                }
                Sink::Text(w) => {
                    let _ = w.flush();
                }
            }
        }
    }

    pub fn finish(&mut self) {
        match &mut self.sink {
            Sink::Pcapng(w) => {
                let _ = w.flush();
            }
            Sink::Text(w) => {
                let _ = w.flush();
            }
        }
    }

    pub fn info(&self) -> CaptureInfo {
        CaptureInfo { file: self.file_name(), format: self.format, frames: self.frames, messages: self.messages, bytes: self.bytes, seconds: self.started.elapsed().as_secs() }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.finish();
    }
}
