//! Wireshark extcap interface.
//!
//! Install by copying (or symlinking) the `keelwire` binary into
//! Wireshark's personal extcap folder (Help → About → Folders). Wireshark then
//! lists "Keelwire (NMEA 2000)" as a capture interface.
//!
//! Source (configurable in Wireshark's interface options):
//! * `http://host:8080` — stream from a running Keelwire (non-exclusive)
//! * any interface URI (`socketcan:can0`, `ipg100:tcp://…`, `sim:` …) — open it directly

use std::fs::OpenOptions;
use std::io::Write;

use keelwire_core::pcap::PcapWriter;
use keelwire_hal::{clock, BusEvent};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub const IFACE: &str = "keelwire";

/// Returns Some(exit code) when the arguments are an extcap invocation.
pub fn maybe_run(args: &[String]) -> Option<i32> {
    if !args.iter().any(|a| a.starts_with("--extcap-") || a == "--capture") {
        return None;
    }
    let get = |k: &str| -> Option<String> {
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == k {
                return it.next().cloned();
            }
            if let Some(v) = a.strip_prefix(&format!("{k}=")) {
                return Some(v.to_string());
            }
        }
        None
    };
    let has = |k: &str| args.iter().any(|a| a == k || a.starts_with(&format!("{k}=")));
    if has("--extcap-interfaces") {
        println!("extcap {{version={}}}{{help=https://github.com/openseastack/keelwire}}", crate::VERSION);
        println!("interface {{value={IFACE}}}{{display=Keelwire (NMEA 2000)}}");
        return Some(0);
    }
    if has("--extcap-dlts") {
        println!("dlt {{number=227}}{{name=CAN_SOCKETCAN}}{{display=SocketCAN (NMEA 2000)}}");
        return Some(0);
    }
    if has("--extcap-config") {
        println!("arg {{number=0}}{{call=--source}}{{display=Source}}{{type=string}}{{default=http://127.0.0.1:8080}}{{tooltip=http://host:port of a running Keelwire, or an interface URI such as socketcan:can0, ipg100:tcp://192.168.1.10, sim:}}");
        println!("arg {{number=1}}{{call=--token}}{{display=API token}}{{type=password}}{{tooltip=Only if the server was started with --token}}");
        return Some(0);
    }
    if has("--capture") {
        let Some(fifo) = get("--fifo") else {
            eprintln!("--fifo required");
            return Some(1);
        };
        let source = get("--source").unwrap_or_else(|| "http://127.0.0.1:8080".into());
        let token = get("--token");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
        let r = rt.block_on(capture(&fifo, &source, token.as_deref()));
        if let Err(e) = r {
            eprintln!("keelwire extcap: {e}");
            return Some(1);
        }
        return Some(0);
    }
    Some(0)
}

async fn capture(fifo: &str, source: &str, token: Option<&str>) -> Result<(), String> {
    if let Some(rest) = source.strip_prefix("http://") {
        return capture_http(fifo, rest, token).await;
    }
    let out = OpenOptions::new().write(true).open(fifo).map_err(|e| format!("open fifo: {e}"))?;
    let mut w = PcapWriter::new(std::io::BufWriter::new(out)).map_err(|e| e.to_string())?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    let _h = keelwire_hal::open(source, tx).map_err(|e| e.to_string())?;
    while let Some(ev) = rx.recv().await {
        if let BusEvent::Frame(f) = ev {
            w.write(clock::to_unix_us(f.timestamp_us), &f).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn capture_http(fifo: &str, hostport: &str, token: Option<&str>) -> Result<(), String> {
    let hp = hostport.trim_end_matches('/');
    let mut s = TcpStream::connect(hp).await.map_err(|e| format!("connect {hp}: {e}"))?;
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!("GET /api/v1/stream/pcap HTTP/1.1\r\nHost: {hp}\r\n{auth}Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).await.map_err(|e| e.to_string())?;
    if !line.contains(" 200 ") {
        return Err(format!("server answered: {}", line.trim()));
    }
    let mut chunked = false;
    loop {
        line.clear();
        r.read_line(&mut line).await.map_err(|e| e.to_string())?;
        let l = line.trim();
        if l.is_empty() {
            break;
        }
        if l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    let mut out = OpenOptions::new().write(true).open(fifo).map_err(|e| format!("open fifo: {e}"))?;
    let mut buf = vec![0u8; 65536];
    if chunked {
        loop {
            line.clear();
            if r.read_line(&mut line).await.map_err(|e| e.to_string())? == 0 {
                return Ok(());
            }
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16).map_err(|_| "bad chunk header".to_string())?;
            if size == 0 {
                return Ok(());
            }
            let mut left = size;
            while left > 0 {
                let n = r.read(&mut buf[..left.min(65536)]).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Ok(());
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                left -= n;
            }
            out.flush().map_err(|e| e.to_string())?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf).await.map_err(|e| e.to_string())?;
        }
    } else {
        loop {
            let n = r.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
    }
}
