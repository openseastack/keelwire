//! `keelwire` command-line entry point.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use keelwire_core::canboat::PgnDb;
use keelwire_core::fastpacket::Reassembler;
use keelwire_hal::BusEvent;
use keelwire_server::{Engine, EngineConfig, ServerOptions, TxMode};

#[derive(Parser, Debug)]
#[command(name = "keelwire", version, about = "Open-source NMEA 2000 analyzer and configuration tool")]
struct Cli {
    /// Run as a headless daemon serving the web UI (default bind 0.0.0.0:8080).
    #[arg(long)]
    headless: bool,
    /// Open this boat file (.keelwire) at start, offline. A bare file path
    /// argument (double-clicked file) does the same.
    #[arg(long)]
    open: Option<PathBuf>,
    /// Don't open the web browser at start (desktop mode opens it by default).
    #[arg(long, env = "KEELWIRE_NO_BROWSER")]
    no_browser: bool,
    /// Bus interface URI to open at start (e.g. socketcan:can0, ipg100:tcp://192.168.1.10, sim:).
    #[arg(short, long, env = "KEELWIRE_INTERFACE")]
    interface: Option<String>,
    /// Web/API port. Default 8080; if 8080 is busy the next free port is used.
    #[arg(short, long, env = "KEELWIRE_PORT")]
    port: Option<u16>,
    /// Full address to bind the HTTP server to (overrides --port), e.g. 0.0.0.0:9000.
    #[arg(long, env = "KEELWIRE_BIND")]
    bind: Option<SocketAddr>,
    /// Require this token for API/WebSocket access (Bearer header or ?token=).
    #[arg(long, env = "KEELWIRE_TOKEN")]
    token: Option<String>,
    /// Allow configuration and control transmissions (they still have to be armed in the UI).
    #[arg(long)]
    allow_tx: bool,
    /// Never transmit anything (no address claim, no discovery requests).
    #[arg(long)]
    passive: bool,
    /// Data directory (captures, settings).
    #[arg(long, env = "KEELWIRE_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Where service sessions are filed, one folder per boat
    /// (default: Documents/Keelwire/Vessels, or <data dir>/Vessels without a Documents folder).
    #[arg(long, env = "KEELWIRE_SESSIONS_DIR")]
    sessions_dir: Option<PathBuf>,
    /// Do not start a service session automatically when a gateway connects.
    #[arg(long, env = "KEELWIRE_NO_AUTO_SESSION")]
    no_auto_session: bool,
    /// Alternative canboat.json PGN database.
    #[arg(long)]
    pgn_db: Option<PathBuf>,
    /// Extra directory of proprietary schema JSON files.
    #[arg(long)]
    schemas: Option<PathBuf>,
    /// Preferred own source address.
    #[arg(long, default_value_t = 128)]
    address: u8,
    /// Ring buffer capacity (messages).
    #[arg(long, default_value_t = 500_000)]
    ring: usize,
    /// Allow any origin (CORS) — for UI development.
    #[arg(long)]
    cors_any: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print bus traffic to stdout.
    Dump {
        #[arg(short, long)]
        interface: String,
        /// Decode messages with the PGN database.
        #[arg(short, long)]
        decode: bool,
        /// Stop after N messages.
        #[arg(short = 'n', long)]
        count: Option<u64>,
        /// Print raw frames instead of reassembled messages.
        #[arg(long)]
        frames: bool,
    },
    /// Decode one message: `decode 127250 00ff7f...`.
    Decode { pgn: u32, data: String },
    /// List interface types.
    Interfaces,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = keelwire_server::extcap::maybe_run(&args) {
        std::process::exit(code);
    }
    // A double-clicked boat file arrives as a bare path argument.
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let boat = args.iter().skip(1).position(|a| a.to_string_lossy().to_ascii_lowercase().ends_with(".keelwire")).map(|i| PathBuf::from(args.remove(i + 1)));
    let mut cli = Cli::parse_from(args);
    if cli.open.is_none() {
        cli.open = boat;
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into())).with_writer(std::io::stderr).init();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let code = rt.block_on(run(cli));
    std::process::exit(code);
}

async fn run(cli: Cli) -> i32 {
    match cli.cmd {
        Some(Cmd::Interfaces) => {
            for k in keelwire_hal::kinds() {
                println!("{:<16} {:<32} {}{}", k.scheme, k.example, k.label, if k.available { String::new() } else { format!("  [unavailable: {}]", k.note) });
            }
            0
        }
        Some(Cmd::Decode { pgn, data }) => {
            let db = PgnDb::embedded();
            let Some(bytes) = keelwire_core::frame::parse_hex(&data) else {
                eprintln!("data must be hex");
                return 2;
            };
            let d = db.decode(pgn, &bytes);
            println!("{}", serde_json::to_string_pretty(&d).unwrap());
            0
        }
        Some(Cmd::Dump { interface, decode, count, frames }) => dump(&interface, decode, count, frames).await,
        None => serve(cli).await,
    }
}

async fn serve(cli: Cli) -> i32 {
    let mut cfg = EngineConfig { preferred_address: cli.address, ring_capacity: cli.ring, allow_tx: cli.allow_tx, pgn_db: cli.pgn_db, schemas_dir: cli.schemas, ..Default::default() };
    if let Some(d) = cli.data_dir {
        cfg.data_dir = d;
    }
    cfg.sessions_dir = cli.sessions_dir.clone().unwrap_or_else(|| keelwire_server::engine::default_sessions_dir(&cfg.data_dir));
    cfg.auto_sessions = !cli.no_auto_session;
    if cli.passive {
        cfg.tx_mode = TxMode::Passive;
    }
    let data_dir = cfg.data_dir.clone();
    let engine = Engine::new(cfg);
    if let Some(p) = &cli.open {
        if let Err(e) = engine.boat_open_path(p) {
            eprintln!("{e}");
            return 2;
        }
    }
    if let Some(i) = &cli.interface {
        if let Err(e) = engine.connect(i) {
            eprintln!("interface {i}: {e}");
            return 2;
        }
    }
    let explicit = cli.bind.is_some() || cli.port.is_some();
    let ip: std::net::IpAddr = if cli.headless { [0, 0, 0, 0].into() } else { [127, 0, 0, 1].into() };
    let bind = cli.bind.unwrap_or_else(|| SocketAddr::new(ip, cli.port.unwrap_or(8080)));
    if cli.headless && cli.token.is_none() && !bind.ip().is_loopback() {
        tracing::warn!("serving on {bind} without --token: anyone on the network can view the bus");
    }
    // An explicit port must be honoured; the default falls through to the next free one.
    let candidates: Vec<SocketAddr> = if explicit { vec![bind] } else { (0..20).map(|i| SocketAddr::new(bind.ip(), bind.port() + i)).collect() };
    let mut served = None;
    let mut last_err = None;
    for a in candidates {
        match keelwire_server::serve(engine.clone(), ServerOptions { bind: a, token: cli.token.clone(), cors_any: cli.cors_any }).await {
            Ok(x) => {
                if a != bind {
                    tracing::warn!("port {} is in use, using {} instead (choose one with --port)", bind.port(), a.port());
                }
                served = Some(x);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let Some((addr, h)) = served else {
        eprintln!("cannot bind {bind}: {}{}", last_err.map(|e| e.to_string()).unwrap_or_default(), if explicit { "" } else { " (try --port <n>)" });
        return 1;
    };
    tracing::info!("Keelwire {} listening on http://{addr} (data: {})", keelwire_server::VERSION, data_dir.display());
    if !cli.headless && !cli.no_browser {
        let host = if addr.ip().is_unspecified() { "127.0.0.1".to_string() } else { addr.ip().to_string() };
        open_browser(&format!("http://{host}:{}/", addr.port()));
    }
    tokio::select! {
        _ = h => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
    0
}

async fn dump(uri: &str, decode: bool, count: Option<u64>, frames: bool) -> i32 {
    let db = if decode { Some(PgnDb::embedded()) } else { None };
    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    let _h = match keelwire_hal::open(uri, tx) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut fp = Reassembler::default();
    if let Some(d) = &db {
        fp.set_fast_pgns(d.fast_pgns());
    }
    let mut n = 0u64;
    let out = std::io::stdout();
    let mut out = out.lock();
    let mut print = |ts: u64, prio: u8, pgn: u32, src: u8, dst: u8, data: &[u8]| -> bool {
        let mut line = format!("{:>12.6} {} {:>6} {:>3} {:>3} [{:>3}] {}", ts as f64 / 1e6, prio, pgn, src, dst, data.len(), keelwire_core::frame::hex(data));
        if let Some(d) = &db {
            let dec = d.decode(pgn, data);
            line.push_str(&format!("  {} | {}", dec.description, dec.summary(8)));
        }
        let _ = writeln!(out, "{line}");
        n += 1;
        count.map(|c| n >= c).unwrap_or(false)
    };
    loop {
        let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(3600), rx.recv()).await else { return 0 };
        let done = match ev {
            BusEvent::Frame(f) => {
                let id = f.id();
                if frames || !fp.is_fast(id.pgn) {
                    print(f.timestamp_us, id.priority, id.pgn, id.source, id.destination, f.payload())
                } else if let Some(a) = fp.push(f.timestamp_us, id.priority, id.pgn, id.source, id.destination, f.payload()) {
                    print(f.timestamp_us, a.priority, a.pgn, a.source, a.destination, &a.data)
                } else {
                    false
                }
            }
            BusEvent::Message(m) => print(m.timestamp_us, m.priority, m.pgn, m.source, m.destination, &m.data),
            BusEvent::Status(s) => {
                eprintln!("# {s:?}");
                matches!(s, keelwire_hal::InterfaceStatus::Finished)
            }
            BusEvent::ErrorFrame { kind, .. } => {
                eprintln!("# error frame {kind:?}");
                false
            }
        };
        if done {
            return 0;
        }
    }
}

/// Open the UI in the default browser so the single binary behaves like a
/// desktop app when double-clicked. Failures are only logged.
fn open_browser(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return; // no desktop session (e.g. SSH to a Pi)
        }
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    if let Err(e) = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        tracing::info!("open {url} in a browser ({e})");
    }
}
