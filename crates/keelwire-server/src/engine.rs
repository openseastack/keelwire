//! The engine: ingest → reassembly → decode → registry/metrics → fan-out,
//! plus network management (own address claim, discovery, request/response
//! correlation) and the transmit guard.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use keelwire_core::canboat::PgnDb;
use keelwire_core::fastpacket::{fragment, Reassembler, SequenceCounter};
use keelwire_core::groupfunction::{GroupFunction, INTERVAL_UNCHANGED, OFFSET_UNCHANGED, PRIORITY_UNCHANGED};
use keelwire_core::iso::{self, ConfigInfo, IsoName, ProductInfo};
use keelwire_core::isotp::{self, TpTracker};
use keelwire_core::metrics::{Metrics, MetricsSnapshot};
use keelwire_core::proprietary::maretron;
use keelwire_core::proprietary::schema::SchemaRegistry;
use keelwire_core::registry::{ChannelLabel, Device, Registry, RegistryEvent};
use keelwire_core::sniffer::{DiffCache, PacketRecord, RingBuffer};
use keelwire_core::{CanId, N2kMessage, RawCanFrame};
use keelwire_hal::{clock, BusEvent, InterfaceCaps, InterfaceHandle, InterfaceStatus, Outbound};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::capture::{Capture, CaptureFormat};

/// How much the engine may transmit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxMode {
    /// Never transmit (pure listener).
    Passive,
    /// Network management only: address claim, ISO requests, heartbeat.
    Query,
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub data_dir: PathBuf,
    pub ring_capacity: usize,
    pub preferred_address: u8,
    pub manufacturer_code: u16,
    pub tx_mode: TxMode,
    pub allow_tx: bool,
    pub schemas_dir: Option<PathBuf>,
    pub pgn_db: Option<PathBuf>,
    /// Where finished service sessions are filed, one folder per boat.
    pub sessions_dir: PathBuf,
    /// Start a service session automatically when a gateway connects.
    pub auto_sessions: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            data_dir: default_data_dir(),
            ring_capacity: 500_000,
            preferred_address: 128,
            manufacturer_code: 2046,
            tx_mode: TxMode::Query,
            allow_tx: false,
            schemas_dir: None,
            pgn_db: None,
            sessions_dir: default_sessions_dir(&default_data_dir()),
            auto_sessions: true,
        }
    }
}

/// `Documents/Keelwire/Vessels` when the user has a Documents folder
/// (desktop), else `<data dir>/Vessels` (headless, Raspberry Pi).
pub fn default_sessions_dir(data_dir: &std::path::Path) -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    match home.map(|h| h.join("Documents")).filter(|d| d.is_dir()) {
        Some(d) => d.join("Keelwire").join("Vessels"),
        None => data_dir.join("Vessels"),
    }
}

pub fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(if cfg!(target_os = "macos") { "Library/Application Support" } else { ".local/share" })))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("keelwire")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    None,
    Claiming,
    Claimed,
    /// The gateway (NGT-1, IPG100) owns the bus address.
    GatewayManaged,
    CannotClaim,
}

#[derive(Clone, Debug, Serialize)]
pub struct Alert {
    pub seq: u64,
    pub ts_us: u64,
    pub severity: &'static str,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Bank {
    pub instance: u8,
    pub source: u8,
    pub device_key: Option<String>,
    /// 0 off, 1 on, 2 error/tripped, 3 n/a — indicators 1..=28
    pub states: Vec<u8>,
    pub currents: Vec<Option<f64>>,
    pub labels: Vec<Option<String>>,
    pub last_us: u64,
}

struct Pending {
    matcher: Box<dyn Fn(&N2kMessage) -> bool + Send>,
    reply: oneshot::Sender<N2kMessage>,
    deadline_us: u64,
}

struct Discovery {
    dst: u8,
    msg: N2kMessage,
    attempts: u8,
}

/// One second of bus-load history for the Overview chart (up to an hour).
#[derive(Clone, Debug, serde::Serialize)]
pub struct LoadPoint {
    /// Unix seconds.
    pub t: u64,
    /// Bus load %, None while no interface was connected (drawn as a gap).
    pub load: Option<f64>,
    pub frames: u32,
}

pub struct State {
    pub load_hist: VecDeque<LoadPoint>,
    load_hist_sec: u64,
    pub db: Arc<PgnDb>,
    pub schemas: SchemaRegistry,
    pub ring: RingBuffer,
    diff: DiffCache,
    pub metrics: Metrics,
    pub traffic: keelwire_core::traffic::Traffic,
    fp: Reassembler,
    tp: TpTracker,
    pub registry: Registry,
    pub alerts: VecDeque<Alert>,
    alert_seq: u64,
    iface: Option<InterfaceHandle>,
    pub iface_status: InterfaceStatus,
    pub caps: InterfaceCaps,
    own_name: IsoName,
    pub own_address: Option<u8>,
    preferred: u8,
    pub claim: ClaimState,
    claim_sent_us: u64,
    pub tx_mode: TxMode,
    pub allow_tx: bool,
    armed_until: Option<u64>,
    pub expert: bool,
    pending: Vec<Pending>,
    discovery: VecDeque<Discovery>,
    seq: SequenceCounter,
    capture: Option<Capture>,
    pub offline: Option<String>,
    pub banks: BTreeMap<u8, Bank>,
    hb_seq: u8,
    last_hb_us: u64,
    frames_tx: broadcast::Sender<RawCanFrame>,
    device_version: Arc<AtomicU64>,
    events_tx: mpsc::Sender<BusEvent>,
    pub data_dir: PathBuf,
    pk_pgns: std::collections::HashSet<u32>,
    // ---- service sessions
    pub ais: crate::session::AisTracker,
    /// Last GNSS fix (lat, lon) and the device that sent it.
    pub last_fix: Option<(f64, f64, u8)>,
    pub session: Option<crate::session::Session>,
    session_cap: Option<Capture>,
    session_last_save_us: u64,
    session_tick: u64,
    pub sessions_dir: PathBuf,
    pub auto_sessions: bool,
    pub technician: Option<String>,
}

pub struct Engine {
    pub state: Mutex<State>,
    pub db: Arc<PgnDb>,
    pub frames: broadcast::Sender<RawCanFrame>,
    pub device_version: Arc<AtomicU64>,
    pub config: EngineConfig,
    /// Native services of the host app (share sheet on iOS), if any.
    pub platform: std::sync::OnceLock<Arc<dyn Platform>>,
}

/// Services the host app provides (implemented by the iOS app; absent in a
/// browser or the headless daemon).
pub trait Platform: Send + Sync {
    /// Present the system share sheet for these files.
    fn share_files(&self, paths: Vec<PathBuf>) -> Result<(), String>;
}

const ARM_DURATION_US: u64 = 15 * 60 * 1_000_000;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    NotAllowed(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Bad(String),
    #[error("timeout waiting for a response from the device")]
    Timeout,
    #[error("{0}")]
    Device(String),
}

impl Engine {
    pub fn new(config: EngineConfig) -> Arc<Engine> {
        let db = Arc::new(match &config.pgn_db {
            Some(p) => std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|s| PgnDb::from_json(&s)).unwrap_or_else(|e| {
                tracing::warn!("could not load PGN database {}: {e}; using embedded copy", p.display());
                PgnDb::embedded()
            }),
            None => PgnDb::embedded(),
        });
        let mut schemas = SchemaRegistry::default();
        let builtin = crate::assets::builtin_schemas();
        for (name, json) in builtin {
            if let Err(e) = schemas.load_json(&json) {
                tracing::warn!("built-in schema {name}: {e}");
            }
        }
        if let Some(dir) = &config.schemas_dir {
            for e in schemas.load_dir(dir) {
                tracing::warn!("schema: {e}");
            }
        }
        let mut fp = Reassembler::default();
        fp.set_fast_pgns(db.fast_pgns());
        let pk_pgns = db
            .by_pgn
            .iter()
            .filter(|(p, _)| **p >= 127000 && !keelwire_core::proprietary::is_proprietary_pgn(**p)) // data PGNs only, not network management
            .filter(|(_, defs)| defs.iter().any(|d| d.fields.iter().any(|f| f.primary_key)))
            .map(|(p, _)| *p)
            .collect();
        let (frames_tx, _) = broadcast::channel(8192);
        let (events_tx, events_rx) = mpsc::channel(8192);
        let device_version = Arc::new(AtomicU64::new(1));
        let unique = host_unique();
        let own_name = IsoName {
            unique_number: unique,
            manufacturer_code: config.manufacturer_code,
            device_function: 130,
            device_class: 25,
            industry_group: 4,
            arbitrary_address_capable: true,
            ..Default::default()
        };
        let _ = std::fs::create_dir_all(config.data_dir.join("captures"));
        let st = State {
            db: db.clone(),
            schemas,
            ring: RingBuffer::new(config.ring_capacity),
            diff: DiffCache::default(),
            metrics: Metrics::new(),
            traffic: keelwire_core::traffic::Traffic::new(),
            fp,
            tp: TpTracker::default(),
            registry: Registry::default(),
            alerts: VecDeque::new(),
            alert_seq: 0,
            iface: None,
            iface_status: InterfaceStatus::Disconnected { reason: "no interface selected".into() },
            caps: InterfaceCaps::default(),
            own_name,
            own_address: None,
            preferred: config.preferred_address,
            claim: ClaimState::None,
            claim_sent_us: 0,
            tx_mode: config.tx_mode,
            allow_tx: config.allow_tx,
            armed_until: None,
            expert: false,
            pending: Vec::new(),
            discovery: VecDeque::new(),
            load_hist: VecDeque::with_capacity(3601),
            load_hist_sec: 0,
            seq: SequenceCounter::default(),
            capture: None,
            offline: None,
            banks: BTreeMap::new(),
            hb_seq: 0,
            last_hb_us: 0,
            frames_tx: frames_tx.clone(),
            device_version: device_version.clone(),
            events_tx,
            data_dir: config.data_dir.clone(),
            pk_pgns,
            ais: Default::default(),
            last_fix: None,
            session: None,
            session_cap: None,
            session_last_save_us: 0,
            session_tick: 0,
            sessions_dir: config.sessions_dir.clone(),
            auto_sessions: config.auto_sessions,
            technician: None,
        };
        let engine = Arc::new(Engine { state: Mutex::new(st), db, frames: frames_tx, device_version, config, platform: std::sync::OnceLock::new() });
        tokio::spawn(ingest(engine.clone(), events_rx));
        tokio::spawn(ticker(engine.clone()));
        engine.load_settings();
        engine.state.lock().session_reopen();
        engine
    }

    // ---------------------------------------------------------------- control

    pub fn connect(&self, uri: &str) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if let Some(old) = st.iface.take() {
            old.stop();
        }
        let h = keelwire_hal::open(uri, st.events_tx.clone()).map_err(|e| EngineError::Bad(e.to_string()))?;
        st.caps = h.caps;
        st.iface = Some(h);
        st.iface_status = InterfaceStatus::Connecting;
        st.offline = None;
        st.claim = ClaimState::None;
        st.own_address = None;
        st.fp = {
            let mut r = Reassembler::default();
            r.set_fast_pgns(st.db.fast_pgns());
            r
        };
        st.tp = TpTracker::default();
        st.push_alert("info", "interface", format!("Connecting to {uri}"));
        Ok(())
    }

    pub fn disconnect(&self) {
        let mut st = self.state.lock();
        if let Some(old) = st.iface.take() {
            old.stop();
        }
        st.iface_status = InterfaceStatus::Disconnected { reason: "disconnected by user".into() };
        st.caps = InterfaceCaps::default();
        st.claim = ClaimState::None;
        st.own_address = None;
        st.session_on_disconnect();
    }

    pub fn iface_uri(&self) -> Option<String> {
        self.state.lock().iface.as_ref().map(|i| i.uri.clone())
    }

    pub fn iface_name(&self) -> Option<String> {
        self.state.lock().iface.as_ref().map(|i| i.name.clone())
    }

    pub fn set_tx(&self, mode: Option<TxMode>, armed: Option<bool>, expert: Option<bool>) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if let Some(m) = mode {
            st.tx_mode = m;
            if m == TxMode::Passive {
                st.armed_until = None;
            }
        }
        if let Some(a) = armed {
            if a {
                if !st.allow_tx {
                    return Err(EngineError::NotAllowed("transmission of configuration/control is disabled on this instance (start with --allow-tx)".into()));
                }
                if st.tx_mode == TxMode::Passive {
                    return Err(EngineError::NotAllowed("switch TX mode from passive to query first".into()));
                }
                st.armed_until = Some(clock::now_us() + ARM_DURATION_US);
                st.push_alert("warning", "tx_armed", "Transmit armed: configuration and control commands will be sent".into());
            } else {
                st.armed_until = None;
            }
        }
        if let Some(e) = expert {
            st.expert = e;
        }
        Ok(())
    }

    pub fn refresh(&self) {
        let mut st = self.state.lock();
        for d in st.registry.devices.values_mut() {
            d.queried = false;
        }
        let req = N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0, 255, iso::iso_request(iso::PGN_ADDRESS_CLAIM));
        let _ = st.send(req, false);
        let addrs: Vec<u8> = st.registry.devices.values().filter(|d| d.name.is_some()).filter_map(|d| d.address).collect();
        for a in addrs {
            st.enqueue_discovery(a);
        }
    }

    // ---------------------------------------------------------------- requests

    /// Send `msg` and wait for the first message matching `matcher`.
    pub async fn request(&self, msg: N2kMessage, armed: bool, timeout: Duration, matcher: Box<dyn Fn(&N2kMessage) -> bool + Send>) -> Result<N2kMessage, EngineError> {
        self.request_opt(Some((msg, armed)), timeout, matcher).await
    }

    /// Wait for a matching message without sending anything.
    pub async fn wait_for(&self, timeout: Duration, matcher: Box<dyn Fn(&N2kMessage) -> bool + Send>) -> Result<N2kMessage, EngineError> {
        self.request_opt(None, timeout, matcher).await
    }

    async fn request_opt(&self, send: Option<(N2kMessage, bool)>, timeout: Duration, matcher: Box<dyn Fn(&N2kMessage) -> bool + Send>) -> Result<N2kMessage, EngineError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut st = self.state.lock();
            st.pending.push(Pending { matcher, reply: tx, deadline_us: clock::now_us() + timeout.as_micros() as u64 });
            if let Some((msg, armed)) = send {
                if let Err(e) = st.send(msg, armed) {
                    st.pending.pop();
                    return Err(e);
                }
            }
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(m)) => Ok(m),
            _ => Err(EngineError::Timeout),
        }
    }

    pub fn device_address(&self, key: &str) -> Result<u8, EngineError> {
        let st = self.state.lock();
        let d = st.registry.get(key).ok_or_else(|| EngineError::NotFound(format!("device {key}")))?;
        d.address.ok_or_else(|| EngineError::Bad("device currently has no address".into()))
    }

    /// 126208 Command + wait for Acknowledge from the device.
    pub async fn group_command(&self, key: &str, pgn: u32, params: Vec<(u8, Vec<u8>)>) -> Result<GroupFunction, EngineError> {
        let dst = self.device_address(key)?;
        let g = GroupFunction::Command { pgn, priority: PRIORITY_UNCHANGED, params };
        self.group_exchange(dst, g, true).await
    }

    pub async fn group_exchange(&self, dst: u8, g: GroupFunction, armed: bool) -> Result<GroupFunction, EngineError> {
        let pgn = g.pgn();
        let msg = N2kMessage::new(3, iso::PGN_GROUP_FUNCTION, 0, dst, g.encode());
        let db = self.db.clone();
        let reply = self
            .request(
                msg,
                armed,
                Duration::from_millis(2500),
                Box::new(move |m| {
                    (m.source == dst && m.pgn == iso::PGN_GROUP_FUNCTION && m.data.first() == Some(&2) && m.data.len() >= 4 && (m.data[1] as u32 | (m.data[2] as u32) << 8 | (m.data[3] as u32) << 16) == pgn)
                        || (m.source == dst && m.pgn == iso::PGN_ISO_ACK && iso::parse_iso_ack(&m.data).map(|(_, p)| p == iso::PGN_GROUP_FUNCTION).unwrap_or(false))
                }),
            )
            .await?;
        if reply.pgn == iso::PGN_ISO_ACK {
            return Err(EngineError::Device("device answered with ISO NAK (group function not supported)".into()));
        }
        let width = |p: u32, f: u8, rest: &[u8]| db.field_bytes(p, f, rest);
        let ack = GroupFunction::decode(&reply.data, &width).ok_or_else(|| EngineError::Device("malformed acknowledge".into()))?;
        if let GroupFunction::Ack { pgn_error, interval_error, ref param_errors, .. } = ack {
            if pgn_error != 0 {
                return Err(EngineError::Device(format!("device rejected: {}", keelwire_core::groupfunction::pgn_error_text(pgn_error))));
            }
            if interval_error != 0 {
                return Err(EngineError::Device(format!("device rejected: {}", keelwire_core::groupfunction::interval_error_text(interval_error))));
            }
            if let Some((i, e)) = param_errors.iter().enumerate().find(|(_, e)| **e != 0) {
                return Err(EngineError::Device(format!("parameter {} rejected: {}", i + 1, keelwire_core::groupfunction::param_error_text(*e))));
            }
        }
        Ok(ack)
    }

    /// ISO Request for `pgn` from the device, waiting for the response.
    pub async fn iso_request(&self, key: &str, pgn: u32) -> Result<N2kMessage, EngineError> {
        let dst = self.device_address(key)?;
        let msg = N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0, dst, iso::iso_request(pgn));
        self.request(
            msg,
            false,
            Duration::from_millis(2000),
            Box::new(move |m| m.source == dst && (m.pgn == pgn || (m.pgn == iso::PGN_ISO_ACK && iso::parse_iso_ack(&m.data).map(|(_, p)| p == pgn).unwrap_or(false)))),
        )
        .await
    }

    pub async fn set_device_instance(&self, key: &str, instance: u8) -> Result<(), EngineError> {
        self.group_command(key, iso::PGN_ADDRESS_CLAIM, vec![(3, vec![instance & 7]), (4, vec![instance >> 3])]).await?;
        let _ = self.iso_request(key, iso::PGN_ADDRESS_CLAIM).await;
        Ok(())
    }

    pub async fn set_system_instance(&self, key: &str, instance: u8) -> Result<(), EngineError> {
        self.group_command(key, iso::PGN_ADDRESS_CLAIM, vec![(8, vec![instance & 0xF])]).await?;
        let _ = self.iso_request(key, iso::PGN_ADDRESS_CLAIM).await;
        Ok(())
    }

    pub async fn set_installation(&self, key: &str, field: u8, text: &str) -> Result<(), EngineError> {
        if text.len() > 70 {
            return Err(EngineError::Bad("installation description is limited to 70 characters".into()));
        }
        self.group_command(key, iso::PGN_CONFIG_INFO, vec![(field, iso::encode_string_lau(text))]).await?;
        let _ = self.iso_request(key, iso::PGN_CONFIG_INFO).await;
        Ok(())
    }

    pub async fn set_pgn_interval(&self, key: &str, pgn: u32, interval_ms: u32) -> Result<(), EngineError> {
        let dst = self.device_address(key)?;
        let g = GroupFunction::Request { pgn, interval_ms, offset_10ms: OFFSET_UNCHANGED, params: vec![] };
        self.group_exchange(dst, g, true).await.map(|_| ())
    }

    pub async fn set_data_instance(&self, key: &str, pgn: u32, field: u8, new: u8) -> Result<(), EngineError> {
        self.group_command(key, pgn, vec![(field, vec![new])]).await.map(|_| ())
    }

    pub fn set_address(&self, key: &str, address: u8) -> Result<(), EngineError> {
        if address > 251 {
            return Err(EngineError::Bad("address must be 0..=251".into()));
        }
        let mut st = self.state.lock();
        let d = st.registry.get(key).ok_or_else(|| EngineError::NotFound(key.into()))?;
        let name = d.name.ok_or_else(|| EngineError::Bad("device has no NAME (no address claim seen)".into()))?;
        let payload = iso::commanded_address(&name, address);
        st.send(N2kMessage::new(6, iso::PGN_COMMANDED_ADDRESS, 0, 255, payload), true)
    }

    /// Switch a load. Sends the standard 127502 Switch Bank Control and, for
    /// Maretron/Carling banks (which may ignore 127502), the 126208 Command on
    /// 127501 that their own tools use, addressed to the bank's device
    /// (parameter 1 = bank instance, parameter n+1 = indicator n).
    pub fn switch(&self, instance: u8, indicator: u8, on: bool) -> Result<(), EngineError> {
        if !(1..=28).contains(&indicator) {
            return Err(EngineError::Bad("indicator must be 1..=28".into()));
        }
        let mut st = self.state.lock();
        st.send(N2kMessage::new(3, 127502, 0, 255, iso::switch_bank_control(instance, &[(indicator, on)])), true)?;
        let target = st.banks.get(&instance).map(|b| b.source).filter(|src| {
            st.registry.by_address(*src).and_then(|d| d.name).map(|n| n.manufacturer_code == maretron::MANUFACTURER_CODE || n.manufacturer_code == 176).unwrap_or(false)
        });
        if let Some(dst) = target {
            let g = GroupFunction::Command { pgn: 127501, priority: PRIORITY_UNCHANGED, params: vec![(1, vec![instance]), (indicator + 1, vec![on as u8])] };
            st.send(N2kMessage::new(3, iso::PGN_GROUP_FUNCTION, 0, dst, g.encode()), true)?;
        }
        Ok(())
    }

    /// Set a tank sender's capacity with the standard 126208 Command on PGN
    /// 127505 field 4 (0.1 L), as confirmed on a Maretron TLM100 by the
    /// signalk-tlm100-config project. Returns whether the new value was seen
    /// in the device's next 127505 broadcast.
    pub async fn set_tank_capacity(&self, key: &str, litres: f64) -> Result<bool, EngineError> {
        if !(1.0..=100_000.0).contains(&litres) {
            return Err(EngineError::Bad("capacity must be between 1 and 100000 litres".into()));
        }
        let dst = self.device_address(key)?;
        let raw = (litres * 10.0).round() as u32;
        let g = GroupFunction::Command { pgn: 127505, priority: 0x0F, params: vec![(4, raw.to_le_bytes().to_vec())] };
        self.group_exchange(dst, g, true).await?;
        let db = self.db.clone();
        let readback = self
            .wait_for(
                Duration::from_secs(6),
                Box::new(move |m| {
                    m.source == dst && m.pgn == 127505 && db.decode(127505, &m.data).get("capacity").and_then(|v| v.as_f64()).map(|c| (c - litres).abs() < 0.5).unwrap_or(false)
                }),
            )
            .await;
        Ok(readback.is_ok())
    }

    /// Set fields of a standard PGN on a device with a 126208 Command
    /// (device settings such as depth offset, fluid type, battery
    /// configuration, instances, sources...). `select` holds primary-key
    /// fields (e.g. instance) identifying which occurrence to change on a
    /// multi-channel device; they are sent first, unchanged. Returns whether
    /// the device's next broadcast shows the new values.
    pub async fn command_fields(&self, key: &str, pgn: u32, select: &[(u8, f64)], values: &[(u8, f64)]) -> Result<bool, EngineError> {
        if values.is_empty() {
            return Err(EngineError::Bad("nothing to set".into()));
        }
        // Selection fields first (unless being changed themselves), then the new values.
        let mut params: Vec<(u8, Vec<u8>)> = Vec::new();
        for (order, v) in select.iter().filter(|(o, _)| !values.iter().any(|(x, _)| x == o)).chain(values) {
            if params.iter().any(|(o, _)| o == order) {
                continue;
            }
            params.push((*order, self.db.encode_param(pgn, *order, *v).map_err(EngineError::Bad)?));
        }
        let dst = self.device_address(key)?;
        let g = GroupFunction::Command { pgn, priority: PRIORITY_UNCHANGED, params };
        self.group_exchange(dst, g, true).await?;
        // Expected values after the change, in decoded units, keyed by field order.
        let db = self.db.clone();
        let def_fields: Vec<(usize, String)> = db.get(pgn).and_then(|d| d.iter().find(|d| !d.fallback)).map(|d| d.fields.iter().map(|f| (f.order, f.id.clone())).collect()).unwrap_or_default();
        let id_of = move |o: u8| def_fields.iter().find(|(x, _)| *x == o as usize).map(|(_, id)| id.clone());
        let step_of = |o: u8| db.get(pgn).and_then(|d| d.iter().find(|d| !d.fallback)).and_then(|d| d.fields.iter().find(|f| f.order == o as usize)).map(|f| if f.resolution == 0.0 { 1.0 } else { f.resolution }).unwrap_or(1.0);
        // Tolerance: half a resolution step.
        let expect: Vec<(String, f64, f64)> = values.iter().filter_map(|(o, v)| id_of(*o).map(|id| (id, *v, step_of(*o) / 2.0 + 1e-9))).collect();
        // A changed key field (e.g. the instance itself) moves the occurrence:
        // match on the new values only.
        let sel: Vec<(String, f64)> = select.iter().filter(|(o, _)| !values.iter().any(|(x, _)| x == o)).filter_map(|(o, v)| id_of(*o).map(|id| (id, *v))).collect();
        let readback = self
            .wait_for(
                Duration::from_secs(8),
                Box::new(move |m| {
                    if m.source != dst || m.pgn != pgn {
                        return false;
                    }
                    let dec = db.decode(pgn, &m.data);
                    let num = |id: &str| dec.get(id).and_then(|v| v.as_f64());
                    sel.iter().all(|(id, v)| num(id).map(|x| (x - v).abs() < 0.5).unwrap_or(false)) && expect.iter().all(|(id, v, tol)| num(id).map(|x| (x - v).abs() <= *tol).unwrap_or(false))
                }),
            )
            .await;
        Ok(readback.is_ok())
    }

    pub fn raw_tx(&self, msg: N2kMessage) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if !st.expert {
            return Err(EngineError::NotAllowed("raw transmission requires expert mode".into()));
        }
        st.send(msg, true)
    }

    /// Send a Maretron command (wrapped in a 126208 Command on 126720, see
    /// `maretron::command_payload`) and collect the device's reply.
    pub async fn maretron_command(&self, key: &str, command: u8, data: Vec<u8>, control: bool) -> Result<MaretronReply, EngineError> {
        let (dst, product_code) = {
            let st = self.state.lock();
            let d = st.registry.get(key).ok_or_else(|| EngineError::NotFound(key.into()))?;
            if d.name.map(|n| n.manufacturer_code) != Some(maretron::MANUFACTURER_CODE) {
                return Err(EngineError::Bad("not a Maretron device".into()));
            }
            let pc = d.product.as_ref().and_then(|p| p.product_code).ok_or_else(|| EngineError::Bad("product code unknown (no Product Information received yet)".into()))?;
            (d.address.ok_or_else(|| EngineError::Bad("device has no address".into()))?, pc)
        };
        let payload = maretron::command_payload(product_code, 1, command, &data);
        let msg = N2kMessage::new(4, iso::PGN_GROUP_FUNCTION, 0, dst, payload);
        let is_response = move |r: &N2kMessage| r.source == dst && r.pgn == maretron::PGN_CONFIG && maretron::ConfigMessage::parse(&r.data).map(|c| c.product_code == product_code).unwrap_or(false);
        // Register both listeners before sending: devices may send the
        // acknowledge and the data reply back to back.
        let (first_tx, first_rx) = oneshot::channel();
        let (data_tx, data_rx) = oneshot::channel();
        {
            let mut st = self.state.lock();
            let deadline = clock::now_us() + 4_000_000;
            st.pending.push(Pending {
                matcher: Box::new(move |r| {
                    is_response(r)
                        || (r.source == dst && r.pgn == iso::PGN_GROUP_FUNCTION && r.data.first() == Some(&2) && r.data.len() >= 5 && (r.data[1] as u32 | (r.data[2] as u32) << 8 | (r.data[3] as u32) << 16) == maretron::PGN_CONFIG)
                        || (r.source == dst && r.pgn == iso::PGN_ISO_ACK && iso::parse_iso_ack(&r.data).map(|(_, p)| p == iso::PGN_GROUP_FUNCTION).unwrap_or(false))
                }),
                reply: first_tx,
                deadline_us: deadline,
            });
            st.pending.push(Pending { matcher: Box::new(is_response), reply: data_tx, deadline_us: deadline });
            if let Err(e) = st.send(msg, control) {
                let n = st.pending.len() - 2;
                st.pending.truncate(n);
                return Err(e);
            }
        }
        let first = match tokio::time::timeout(Duration::from_millis(3000), first_rx).await {
            Ok(Ok(m)) => m,
            _ => return Err(EngineError::Timeout),
        };
        if first.pgn == iso::PGN_ISO_ACK {
            return Err(EngineError::Device("device answered with ISO NAK (command not accepted)".into()));
        }
        if first.pgn == iso::PGN_GROUP_FUNCTION && first.data[4] & 0x0F != 0 {
            return Err(EngineError::Device(format!("device rejected: {}", keelwire_core::groupfunction::pgn_error_text(first.data[4] & 0x0F))));
        }
        let data_msg = if first.pgn == maretron::PGN_CONFIG { Some(first) } else { tokio::time::timeout(Duration::from_millis(1500), data_rx).await.ok().and_then(|r| r.ok()) };
        Ok(MaretronReply {
            acknowledged: true,
            decoded: data_msg.as_ref().map(|m| self.db.decode(maretron::PGN_CONFIG, &m.data)),
            response: data_msg.and_then(|m| maretron::ConfigMessage::parse(&m.data)),
        })
    }

    /// Run a named Maretron action (see `crate::maretron_actions`).
    pub async fn maretron_action(&self, key: &str, id: &str, value: Option<f64>, value2: Option<f64>) -> Result<MaretronReply, EngineError> {
        let product = self.device(key).and_then(|d| d.product.and_then(|p| p.product_code)).unwrap_or(0);
        let info = crate::maretron_actions::find(id, product).ok_or_else(|| EngineError::Bad(format!("action '{id}' is not available for this device")))?;
        if info.experimental && !self.state.lock().expert {
            return Err(EngineError::NotAllowed("this action uses a layout that has not been confirmed on hardware; enable expert mode to use it".into()));
        }
        let (command, data) = crate::maretron_actions::encode(id, value, value2).map_err(EngineError::Bad)?;
        self.maretron_command(key, command, data, info.control).await
    }

    // ---------------------------------------------------------------- views

    pub fn devices(&self) -> Vec<Device> {
        self.state.lock().registry.list().into_iter().cloned().collect()
    }

    /// Current key of the device at `address` (keys change when a NAME changes).
    pub fn key_at(&self, address: u8) -> Option<String> {
        self.state.lock().registry.by_address(address).map(|d| d.key.clone())
    }

    pub fn device(&self, key: &str) -> Option<Device> {
        self.state.lock().registry.get(key).cloned()
    }

    pub fn metrics(&self) -> MetricsSnapshot {
        let mut st = self.state.lock();
        st.metrics.snapshot(clock::now_us())
    }

    /// Cumulative traffic per sender and per (PGN, sender), and engine time now.
    pub fn traffic(&self) -> (keelwire_core::traffic::TrafficSnapshot, u64) {
        let st = self.state.lock();
        (st.traffic.snapshot(), clock::now_us())
    }

    pub fn reset_traffic(&self) {
        let mut st = self.state.lock();
        let inc = st.fp.incomplete_by_source.clone();
        st.traffic.reset(clock::now_us(), &inc);
    }

    pub fn alerts_after(&self, seq: u64) -> Vec<Alert> {
        self.state.lock().alerts.iter().filter(|a| a.seq > seq).cloned().collect()
    }

    pub fn packets_after(&self, seq: u64, limit: usize) -> Vec<PacketRecord> {
        self.state.lock().ring.after(seq, limit)
    }

    pub fn packets_tail(&self, n: usize) -> Vec<PacketRecord> {
        self.state.lock().ring.tail(n)
    }

    pub fn banks(&self) -> Vec<Bank> {
        self.state.lock().banks.values().cloned().collect()
    }

    pub fn is_armed(&self) -> bool {
        self.state.lock().armed()
    }

    pub fn clear_packets(&self) {
        let mut st = self.state.lock();
        st.ring.clear();
        st.diff.clear();
    }

    pub fn analysis(&self) -> Vec<keelwire_core::analysis::Finding> {
        let st = self.state.lock();
        let devs = st.registry.list();
        let db = &st.db;
        keelwire_core::analysis::analyze(&devs, &|p| db.name(p).unwrap_or("PGN").to_string())
    }

    pub fn load_history(&self, seconds: usize) -> Vec<LoadPoint> {
        let st = self.state.lock();
        let n = st.load_hist.len();
        st.load_hist.iter().skip(n.saturating_sub(seconds)).cloned().collect()
    }

    pub fn status(&self) -> StatusDto {
        let st = self.state.lock();
        StatusDto {
            version: crate::VERSION,
            interface: st.iface.as_ref().map(|i| InterfaceDto { uri: i.uri.clone(), name: i.name.clone() }),
            interface_status: st.iface_status.clone(),
            caps: st.caps,
            own_name: st.own_name.hex(),
            own_address: st.own_address,
            claim: st.claim,
            tx_mode: st.tx_mode,
            allow_tx: st.allow_tx,
            armed: st.armed(),
            armed_remaining_s: st.armed_until.map(|u| u.saturating_sub(clock::now_us()) / 1_000_000),
            expert: st.expert,
            offline: st.offline.clone(),
            capture: st.capture.as_ref().map(|c| c.info()),
            pgn_db_version: st.db.version.clone(),
            schemas: st.schemas.schemas.len(),
            ring: (st.ring.len(), st.ring.capacity()),
            last_seq: st.ring.last_seq().unwrap_or(0),
            units: keelwire_core::units::current(),
            platform: std::env::consts::OS,
            can_share: self.platform.get().is_some(),
        }
    }

    // ---------------------------------------------------------------- settings

    fn settings_path(&self) -> PathBuf {
        self.state.lock().data_dir.join("settings.json")
    }

    /// Persisted user settings (display units). Unknown keys are kept.
    pub fn load_settings(&self) {
        if let Ok(t) = std::fs::read_to_string(self.settings_path()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                if let Some(u) = v.get("units").and_then(|u| serde_json::from_value(u.clone()).ok()) {
                    keelwire_core::units::set(u);
                }
                if let Some(t) = v.get("technician").and_then(|t| t.as_str()) {
                    self.state.lock().technician = Some(t.to_string()).filter(|t| !t.is_empty());
                }
            }
        }
    }

    pub fn set_units(&self, u: keelwire_core::units::Units) -> Result<(), EngineError> {
        let path = self.settings_path();
        let mut v: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
        v["units"] = serde_json::to_value(&u).map_err(|e| EngineError::Bad(e.to_string()))?;
        std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default()).map_err(|e| EngineError::Bad(format!("cannot save settings: {e}")))?;
        keelwire_core::units::set(u);
        Ok(())
    }

    // ---------------------------------------------------------------- capture

    pub fn capture_start(&self, format: CaptureFormat, name: Option<String>) -> Result<String, EngineError> {
        let mut st = self.state.lock();
        if st.capture.is_some() {
            return Err(EngineError::Bad("a capture is already running".into()));
        }
        let dir = st.data_dir.join("captures");
        let iface = st.iface.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| "n2k".into());
        let c = Capture::start(&dir, format, name, &iface).map_err(|e| EngineError::Bad(e.to_string()))?;
        let file = c.file_name();
        st.capture = Some(c);
        Ok(file)
    }

    pub fn capture_stop(&self) -> Option<crate::capture::CaptureInfo> {
        let mut st = self.state.lock();
        st.capture.take().map(|mut c| {
            c.finish();
            c.info()
        })
    }

    // ---------------------------------------------------------------- boat files

    pub fn boat_snapshot(&self, vessel: Option<String>) -> crate::boatfile::BoatFile {
        let st = self.state.lock();
        crate::boatfile::BoatFile::from_state(&st, vessel)
    }

    pub fn boat_load(&self, b: crate::boatfile::BoatFile, name: String) {
        let mut st = self.state.lock();
        if let Some(old) = st.iface.take() {
            old.stop();
        }
        st.iface_status = InterfaceStatus::Disconnected { reason: "offline (boat file)".into() };
        st.caps = InterfaceCaps::default();
        st.claim = ClaimState::None;
        st.own_address = None;
        st.registry.clear();
        st.ring.clear();
        st.diff.clear();
        st.banks.clear();
        b.apply(&mut st);
        st.offline = Some(name.clone());
        st.push_alert("info", "offline", format!("Loaded boat file {name} (offline mode)"));
        st.bump();
    }

    /// Open a boat file from disk (double-clicked file / command line).
    pub fn boat_open_path(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let b: crate::boatfile::BoatFile = serde_json::from_slice(&bytes).map_err(|e| format!("{}: not a boat file ({e})", path.display()))?;
        b.validate()?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "boat file".into());
        self.boat_load(b, name);
        Ok(())
    }

    pub fn boat_close(&self) {
        let mut st = self.state.lock();
        st.offline = None;
        st.registry.clear();
        st.banks.clear();
        st.ring.clear();
        st.bump();
    }
}

#[derive(Serialize)]
pub struct MaretronReply {
    /// The device accepted the request (126208 Acknowledge or a data reply).
    pub acknowledged: bool,
    pub response: Option<maretron::ConfigMessage>,
    pub decoded: Option<keelwire_core::canboat::Decoded>,
}

#[derive(Serialize)]
pub struct InterfaceDto {
    pub uri: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct StatusDto {
    pub version: &'static str,
    pub interface: Option<InterfaceDto>,
    pub interface_status: InterfaceStatus,
    pub caps: InterfaceCaps,
    pub own_name: String,
    pub own_address: Option<u8>,
    pub claim: ClaimState,
    pub tx_mode: TxMode,
    pub allow_tx: bool,
    pub armed: bool,
    pub armed_remaining_s: Option<u64>,
    pub expert: bool,
    pub offline: Option<String>,
    pub capture: Option<crate::capture::CaptureInfo>,
    pub pgn_db_version: String,
    pub schemas: usize,
    pub ring: (usize, usize),
    pub last_seq: u64,
    pub units: keelwire_core::units::Units,
    /// "ios", "android", "macos", "windows", "linux"…
    pub platform: &'static str,
    /// The host app can present a share sheet (`POST /native/share`).
    pub can_share: bool,
}

fn host_unique() -> u32 {
    let host = std::env::var("HOSTNAME").or_else(|_| std::fs::read_to_string("/etc/hostname")).unwrap_or_else(|_| "keelwire".into());
    let mut h: u32 = 2_166_136_261;
    for b in host.trim().bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    h & 0x1F_FFFF
}

// -------------------------------------------------------------------- state

impl State {
    pub fn bump(&self) {
        self.device_version.fetch_add(1, Ordering::Relaxed);
    }

    pub fn armed(&self) -> bool {
        self.armed_until.map(|u| u > clock::now_us()).unwrap_or(false)
    }

    pub fn push_alert(&mut self, severity: &'static str, kind: &str, message: String) {
        self.alert_seq += 1;
        self.alerts.push_back(Alert { seq: self.alert_seq, ts_us: clock::now_us(), severity, kind: kind.into(), message });
        while self.alerts.len() > 500 {
            self.alerts.pop_front();
        }
    }

    fn is_fast(&self, pgn: u32) -> bool {
        self.fp.is_fast(pgn)
    }

    fn can_query(&self) -> bool {
        self.tx_mode == TxMode::Query && self.caps.can_transmit && self.offline.is_none() && matches!(self.claim, ClaimState::Claimed | ClaimState::GatewayManaged)
    }

    /// Transmit a message. `control` = state-changing (requires arming).
    pub fn send(&mut self, mut msg: N2kMessage, control: bool) -> Result<(), EngineError> {
        if self.offline.is_some() {
            return Err(EngineError::NotAllowed("offline (boat file loaded)".into()));
        }
        let Some(iface) = self.iface.as_ref() else { return Err(EngineError::NotAllowed("no interface connected".into())) };
        if !self.caps.can_transmit {
            return Err(EngineError::NotAllowed(format!("{} cannot transmit", iface.name)));
        }
        if self.tx_mode == TxMode::Passive {
            return Err(EngineError::NotAllowed("TX mode is passive".into()));
        }
        if control && !self.armed() {
            return Err(EngineError::NotAllowed("transmit is not armed (Settings → Arm TX)".into()));
        }
        let is_claim = msg.pgn == iso::PGN_ADDRESS_CLAIM;
        msg.source = match self.claim {
            ClaimState::Claimed => self.own_address.unwrap_or(254),
            ClaimState::GatewayManaged => 0xFF,
            _ if is_claim => msg.source,
            ClaimState::Claiming if msg.pgn == iso::PGN_ISO_REQUEST => 254,
            _ => return Err(EngineError::NotAllowed("own address not claimed yet".into())),
        };
        if let Some(a) = self.armed_until.as_mut() {
            if control {
                *a = clock::now_us() + ARM_DURATION_US; // activity extends arming
            }
        }
        let now = clock::now_us();
        msg.timestamp_us = now;
        let tx = iface.tx.clone();
        let mut frames: Vec<RawCanFrame> = Vec::new();
        let id = CanId { priority: msg.priority, pgn: msg.pgn, destination: msg.destination, source: msg.source }.encode();
        if self.caps.tx_messages {
            tx.try_send(Outbound::Message(msg.clone())).map_err(|e| EngineError::Bad(format!("interface queue: {e}")))?;
            if msg.data.len() <= 8 {
                frames.push(RawCanFrame::new(now, id, &msg.data));
            } else {
                let s = self.seq.next(msg.pgn);
                frames.extend(fragment(s, &msg.data[..msg.data.len().min(223)]).iter().map(|f| RawCanFrame::new(now, id, f)));
            }
        } else if msg.data.len() <= 8 && !self.is_fast(msg.pgn) {
            frames.push(RawCanFrame::new(now, id, &msg.data));
        } else if self.is_fast(msg.pgn) && msg.data.len() <= 223 {
            let s = self.seq.next(msg.pgn);
            frames.extend(fragment(s, &msg.data).iter().map(|f| RawCanFrame::new(now, id, f)));
        } else if msg.destination == 255 && msg.data.len() <= isotp::TP_MAX {
            // ISO TP BAM, 50 ms between packets (sent from a task).
            let bam = isotp::bam_frames(msg.pgn, &msg.data);
            let src = msg.source;
            let prio = msg.priority.max(6);
            let ftx = self.frames_tx.clone();
            tokio::spawn(async move {
                for (i, (pgn, data)) in bam.into_iter().enumerate() {
                    if i > 0 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    let f = RawCanFrame::new(clock::now_us(), CanId { priority: prio, pgn, destination: 255, source: src }.encode(), &data);
                    let _ = ftx.send(f.clone());
                    let _ = tx.send(Outbound::Frame(f)).await;
                }
            });
            self.record(msg, 0, true);
            return Ok(());
        } else {
            return Err(EngineError::Bad("message too long for this destination".into()));
        }
        if !self.caps.tx_messages {
            for f in &frames {
                tx.try_send(Outbound::Frame(f.clone())).map_err(|e| EngineError::Bad(format!("interface queue: {e}")))?;
            }
        }
        let unix = clock::to_unix_us(now);
        for f in &frames {
            let _ = self.frames_tx.send(f.clone());
            self.cap_frame(unix, f);
            self.metrics.record_frame(now, msg.source, f.dlc);
            self.traffic.record_frame(now, msg.source, f.dlc);
        }
        let n = frames.len() as u8;
        if let Some(c) = self.capture.as_mut() {
            c.write_message(unix, &msg);
        }
        if let Some(c) = self.session_cap.as_mut() {
            c.write_message(unix, &msg);
        }
        self.record(msg, n, true);
        Ok(())
    }

    fn record(&mut self, m: N2kMessage, frames: u8, tx: bool) {
        let diff = self.diff.diff(m.pgn, m.source, &m.data);
        self.metrics.record_message(m.timestamp_us, m.pgn, m.source);
        self.traffic.record_message(m.timestamp_us, m.pgn, m.source, frames, m.data.len());
        self.ring.push(PacketRecord { seq: 0, timestamp_us: m.timestamp_us, priority: m.priority, pgn: m.pgn, source: m.source, destination: m.destination, data: m.data, frames, tx, diff });
    }

    fn enqueue_discovery(&mut self, dst: u8) {
        if let Some(d) = self.registry.by_address_mut(dst) {
            if d.queried {
                return;
            }
            d.queried = true;
        }
        for pgn in [iso::PGN_PRODUCT_INFO, iso::PGN_CONFIG_INFO] {
            self.discovery.push_back(Discovery { dst, msg: N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0, dst, iso::iso_request(pgn)), attempts: 0 });
        }
        for which in [0u8, 1] {
            let g = GroupFunction::Request { pgn: iso::PGN_PGN_LIST, interval_ms: INTERVAL_UNCHANGED, offset_10ms: OFFSET_UNCHANGED, params: vec![(1, vec![which])] };
            self.discovery.push_back(Discovery { dst, msg: N2kMessage::new(6, iso::PGN_GROUP_FUNCTION, 0, dst, g.encode()), attempts: 0 });
        }
    }

    fn send_claim(&mut self, address: u8) {
        let mut m = N2kMessage::new(6, iso::PGN_ADDRESS_CLAIM, address, 255, self.own_name.to_bytes().to_vec());
        m.source = address;
        let prev = self.claim;
        self.claim = ClaimState::Claiming;
        self.own_address = Some(address);
        if let Err(e) = self.send(m, false) {
            tracing::debug!("claim not sent: {e}");
            self.claim = prev;
        }
        self.claim_sent_us = clock::now_us();
    }

    fn start_network(&mut self) {
        if self.tx_mode != TxMode::Query || !self.caps.can_transmit || self.offline.is_some() {
            return;
        }
        if self.caps.tx_messages {
            self.claim = ClaimState::GatewayManaged;
            self.own_address = None;
            let req = N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0, 255, iso::iso_request(iso::PGN_ADDRESS_CLAIM));
            let _ = self.send(req, false);
        } else {
            let a = self.preferred;
            self.send_claim(a);
        }
    }

    // ------------------------------------------------------------- ingest

    fn on_event(&mut self, ev: BusEvent) {
        match ev {
            BusEvent::Frame(f) => self.on_frame(f),
            BusEvent::Message(m) => self.on_gateway_message(m),
            BusEvent::ErrorFrame { timestamp_us, kind } => {
                self.metrics.record_error(timestamp_us);
                if kind == keelwire_hal::BusErrorKind::BusOff {
                    self.push_alert("error", "bus_off", "CAN controller is bus-off".into());
                }
            }
            BusEvent::Status(s) => {
                let connected = matches!(s, InterfaceStatus::Connected { .. });
                match &s {
                    InterfaceStatus::Connected { detail } => self.push_alert("info", "interface", format!("Connected: {detail}")),
                    InterfaceStatus::Disconnected { reason } => self.push_alert("warning", "interface", format!("Disconnected: {reason}")),
                    InterfaceStatus::Finished => self.push_alert("info", "interface", "Replay finished".into()),
                    _ => {}
                }
                self.iface_status = s;
                if connected {
                    self.start_network();
                    self.session_on_connect();
                } else {
                    self.claim = ClaimState::None;
                    self.session_on_disconnect();
                }
            }
        }
    }

    fn on_frame(&mut self, f: RawCanFrame) {
        let ts = f.timestamp_us;
        let id = f.id();
        let _ = self.frames_tx.send(f.clone());
        self.cap_frame(clock::to_unix_us(ts), &f);
        self.metrics.record_frame(ts, id.source, f.dlc);
        self.traffic.record_frame(ts, id.source, f.dlc);
        let (_, ev) = self.registry.touch(ts, id.source);
        if ev.is_some() {
            self.bump();
        }
        let data = f.payload();
        if id.pgn == isotp::PGN_TP_CM || id.pgn == isotp::PGN_TP_DT {
            self.tp.own_address = if self.claim == ClaimState::Claimed { self.own_address } else { None };
            let (m, replies) = self.tp.push(ts, id.priority, id.pgn, id.source, id.destination, data);
            for r in replies {
                let mut msg = N2kMessage::new(7, r.pgn, 0, r.destination, r.data.to_vec());
                msg.priority = 7;
                let _ = self.send(msg, false);
            }
            if let Some(m) = m {
                let n = m.data.len().div_ceil(7) + 1;
                self.on_message(N2kMessage { timestamp_us: ts, priority: m.priority, pgn: m.pgn, source: m.source, destination: m.destination, data: m.data }, n as u8);
            }
            return;
        }
        if self.is_fast(id.pgn) {
            if let Some(a) = self.fp.push(ts, id.priority, id.pgn, id.source, id.destination, data) {
                let n = if a.data.len() <= 6 { 1 } else { 1 + (a.data.len() - 6).div_ceil(7) };
                self.on_message(N2kMessage { timestamp_us: ts, priority: a.priority, pgn: a.pgn, source: a.source, destination: a.destination, data: a.data }, n as u8);
            }
            return;
        }
        self.on_message(N2kMessage { timestamp_us: ts, priority: id.priority, pgn: id.pgn, source: id.source, destination: id.destination, data: data.to_vec() }, 1);
    }

    fn on_gateway_message(&mut self, m: N2kMessage) {
        let ts = m.timestamp_us;
        let id = m.can_id().encode();
        let unix = clock::to_unix_us(ts);
        let frames: Vec<RawCanFrame> = if m.data.len() <= 8 && !self.is_fast(m.pgn) {
            vec![RawCanFrame::new(ts, id, &m.data)]
        } else {
            let s = self.seq.next(m.pgn ^ 0x8000_0000);
            fragment(s, &m.data[..m.data.len().min(223)]).iter().map(|f| RawCanFrame::new(ts, id, f)).collect()
        };
        for f in &frames {
            let _ = self.frames_tx.send(f.clone());
            self.cap_frame(unix, f);
            self.metrics.record_frame(ts, m.source, f.dlc);
            self.traffic.record_frame(ts, m.source, f.dlc);
        }
        let (_, ev) = self.registry.touch(ts, m.source);
        if ev.is_some() {
            self.bump();
        }
        self.on_message(m, 0);
    }

    fn on_message(&mut self, m: N2kMessage, frames: u8) {
        if let Some(c) = self.capture.as_mut() {
            c.write_message(clock::to_unix_us(m.timestamp_us), &m);
        }
        if let Some(c) = self.session_cap.as_mut() {
            c.write_message(clock::to_unix_us(m.timestamp_us), &m);
        }
        self.session_observe(&m);
        // Observed-PGN bookkeeping, incl. primary-key signatures for the instance analyzer.
        let want_pk = self.pk_pgns.contains(&m.pgn);
        let db = self.db.clone();
        if let Some(d) = self.registry.by_address_mut(m.source) {
            let o = d.observed.entry(m.pgn).or_default();
            o.count += 1;
            o.last_us = m.timestamp_us;
            o.destination = m.destination;
            o.last_data = m.data[..m.data.len().min(256)].to_vec();
            if want_pk {
                let dec = db.decode(m.pgn, &m.data);
                if let Some(k) = db.primary_key(&dec) {
                    // A signature not repeated for 30 s while the device keeps
                    // sending this PGN under another one is gone (e.g. after an
                    // instance change), over a 30-second window.
                    o.note_key_data(k, m.timestamp_us, 30_000_000, &m.data);
                }
            }
        }
        // Resolve pending requests.
        if !self.pending.is_empty() {
            let mut i = 0;
            while i < self.pending.len() {
                if (self.pending[i].matcher)(&m) {
                    let p = self.pending.swap_remove(i);
                    let _ = p.reply.send(m.clone());
                } else {
                    i += 1;
                }
            }
        }
        self.network_management(&m);
        self.record(m, frames, false);
    }

    fn network_management(&mut self, m: &N2kMessage) {
        let ts = m.timestamp_us;
        match m.pgn {
            iso::PGN_ADDRESS_CLAIM => {
                let Some(name) = IsoName::from_bytes(&m.data) else { return };
                // Contention with us?
                if (self.claim == ClaimState::Claimed || self.claim == ClaimState::Claiming)
                    && Some(m.source) == self.own_address && name != self.own_name {
                        if self.own_name.to_u64() < name.to_u64() {
                            let a = m.source;
                            self.send_claim(a); // defend
                            self.claim = ClaimState::Claimed;
                        } else {
                            let used: Vec<u8> = self.registry.devices.values().filter_map(|d| d.address).collect();
                            match (self.preferred..=251).chain(0..self.preferred).find(|a| !used.contains(a) && *a != m.source) {
                                Some(a) => {
                                    self.push_alert("warning", "own_address", format!("Lost address {} to another device; claiming {a}", m.source));
                                    self.send_claim(a);
                                }
                                None => {
                                    self.claim = ClaimState::CannotClaim;
                                    self.push_alert("error", "own_address", "No free address available".into());
                                }
                            }
                        }
                    }
                let evs = self.registry.claim(ts, m.source, name.to_u64());
                let key = name.hex();
                let db = self.db.clone();
                if let Some(d) = self.registry.get_mut(&key) {
                    d.manufacturer = db.manufacturer(name.manufacturer_code).map(|s| s.to_string());
                    d.device_class_text = db.device_class(name.device_class).map(|s| s.to_string());
                    d.device_function_text = db.device_function(name.device_class, name.device_function).map(|s| s.to_string());
                }
                for e in evs {
                    match e {
                        RegistryEvent::NewDevice { address, .. } => {
                            if self.can_query() {
                                self.enqueue_discovery(address);
                            }
                        }
                        RegistryEvent::AddressChanged { key, old, new } => self.push_alert("info", "address_change", format!("{key} moved from address {old} to {new}")),
                        RegistryEvent::Contention { address, winner, loser } => self.push_alert("warning", "address_contention", format!("Address {address} contention: {winner} kept it, {loser} must move")),
                        RegistryEvent::DuplicateAddress { address, keys } => self.push_alert("error", "duplicate_address", format!("Duplicate source address {address}: {}", keys.join(" and "))),
                        RegistryEvent::CannotClaim { key } => self.push_alert("error", "cannot_claim", format!("{key} could not claim an address")),
                        RegistryEvent::Renamed { old, new } => self.push_alert("info", "name_changed", format!("Device {old} now identifies as {new} (instance changed)")),
                        _ => {}
                    }
                }
                self.bump();
            }
            iso::PGN_ISO_REQUEST => {
                let Some(p) = iso::parse_iso_request(&m.data) else { return };
                let to_us = m.destination == 255 || (self.claim == ClaimState::Claimed && Some(m.destination) == self.own_address);
                if !to_us || self.claim != ClaimState::Claimed {
                    return;
                }
                let dst = if m.destination == 255 { 255 } else { m.source };
                let reply = match p {
                    iso::PGN_ADDRESS_CLAIM => Some(N2kMessage::new(6, p, 0, 255, self.own_name.to_bytes().to_vec())),
                    iso::PGN_PRODUCT_INFO => Some(N2kMessage::new(6, p, 0, dst, own_product().encode())),
                    iso::PGN_PGN_LIST => {
                        let _ = self.send(N2kMessage::new(6, p, 0, dst, iso::encode_pgn_list(0, &[59392, 59904, 60928, 126208, 126464, 126993, 126996])), false);
                        Some(N2kMessage::new(6, p, 0, dst, iso::encode_pgn_list(1, &[59392, 59904, 60928, 126208, 126464, 126996, 126998])))
                    }
                    _ if m.destination != 255 => Some(N2kMessage::new(6, iso::PGN_ISO_ACK, 0, m.source, iso::iso_ack(1, p))),
                    _ => None,
                };
                if let Some(r) = reply {
                    let _ = self.send(r, false);
                }
            }
            iso::PGN_PRODUCT_INFO => {
                if let (Some(p), Some(d)) = (ProductInfo::parse(&m.data), self.registry.by_address_mut(m.source)) {
                    let maretron = d.name.map(|n| n.manufacturer_code == maretron::MANUFACTURER_CODE).unwrap_or(false);
                    d.model = Some(if maretron { p.product_code.and_then(maretron::product_name).map(|s| s.to_string()).unwrap_or_else(|| p.model_id.clone()) } else { p.model_id.clone() });
                    d.product = Some(p);
                    self.bump();
                }
            }
            iso::PGN_CONFIG_INFO => {
                if let (Some(c), Some(d)) = (ConfigInfo::parse(&m.data), self.registry.by_address_mut(m.source)) {
                    d.config = Some(c);
                    self.bump();
                }
            }
            iso::PGN_PGN_LIST => {
                if let (Some((f, list)), Some(d)) = (iso::parse_pgn_list(&m.data), self.registry.by_address_mut(m.source)) {
                    match f {
                        0 => d.tx_pgns = Some(list),
                        1 => d.rx_pgns = Some(list),
                        _ => {}
                    }
                    self.bump();
                }
            }
            iso::PGN_HEARTBEAT => {
                if let (Some(h), Some(d)) = (iso::Heartbeat::parse(&m.data), self.registry.by_address_mut(m.source)) {
                    d.heartbeat_interval_ms = h.interval_ms;
                    d.heartbeat_last_us = Some(ts);
                }
            }
            maretron::PGN_LABEL => {
                if let (Some(l), Some(d)) = (maretron::Label::parse(&m.data), self.registry.by_address_mut(m.source)) {
                    let entry = ChannelLabel { instance: l.instance, data_source: l.data_source, indicator: l.data_indicator, hardware_channel: l.hardware_channel, label: l.label.clone() };
                    d.labels.retain(|x| !(x.instance == entry.instance && x.indicator == entry.indicator && x.data_source == entry.data_source));
                    d.labels.push(entry);
                    let inst = l.instance;
                    if let Some(b) = self.banks.get_mut(&inst) {
                        if l.data_indicator >= 1 && (l.data_indicator as usize) <= b.labels.len() {
                            b.labels[l.data_indicator as usize - 1] = Some(l.label);
                        }
                    }
                    self.bump();
                }
            }
            127501 => {
                if let Some((inst, st)) = iso::parse_switch_bank(&m.data) {
                    let key = self.registry.by_address(m.source).map(|d| d.key.clone());
                    let labels = self
                        .registry
                        .by_address(m.source)
                        .map(|d| (1..=28u8).map(|i| d.labels.iter().find(|l| l.instance == inst && l.indicator == i).map(|l| l.label.clone())).collect::<Vec<_>>())
                        .unwrap_or_else(|| vec![None; 28]);
                    let b = self.banks.entry(inst).or_insert_with(|| Bank { instance: inst, currents: vec![None; 28], labels: vec![None; 28], ..Default::default() });
                    b.source = m.source;
                    b.device_key = key;
                    b.states = st.to_vec();
                    b.last_us = ts;
                    for (i, l) in labels.into_iter().enumerate() {
                        if l.is_some() {
                            b.labels[i] = l;
                        }
                    }
                }
            }
            maretron::PGN_BREAKER_CURRENT => {
                if let Some((inst, ind, amps)) = maretron::parse_breaker_current(&m.data) {
                    let b = self.banks.entry(inst).or_insert_with(|| Bank { instance: inst, source: m.source, states: vec![3; 28], currents: vec![None; 28], labels: vec![None; 28], ..Default::default() });
                    if (1..=28).contains(&ind) {
                        b.currents[ind as usize - 1] = amps;
                    }
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        let now = clock::now_us();
        let unix_s = clock::unix_now_us() / 1_000_000;
        if unix_s != self.load_hist_sec {
            self.load_hist_sec = unix_s;
            let connected = matches!(self.iface_status, InterfaceStatus::Connected { .. }) || self.offline.is_some();
            let (load, frames) = match self.metrics.last_second() {
                Some(l) if connected && l.t + 2 >= now / 1_000_000 => (Some(l.load_pct), l.frames),
                _ if connected => (Some(0.0), 0),
                _ => (None, 0),
            };
            self.load_hist.push_back(LoadPoint { t: unix_s, load, frames });
            while self.load_hist.len() > 3600 {
                self.load_hist.pop_front();
            }
        }
        let n = self.fp.expire(now);
        self.metrics.fp_timeouts += n as u64;
        self.metrics.fp_orphans = self.fp.stats.orphans;
        self.traffic.set_incomplete(&self.fp.incomplete_by_source);
        let t = self.tp.expire(now);
        self.metrics.tp_aborts += t as u64;
        for e in self.registry.sweep(now) {
            if let RegistryEvent::Offline { key } = e {
                self.push_alert("warning", "offline", format!("{key} went offline"));
                let label = self.registry.get(&key).map(|d| crate::session::DeviceSnap::from_device(d).label()).unwrap_or(key.clone());
                if let Some(s) = self.session.as_mut().filter(|s| !s.is_paused()) {
                    s.event("offline", format!("{label} stopped responding."));
                }
                self.bump();
            }
        }
        // pending timeouts
        self.pending.retain(|p| p.deadline_us > now);
        // claim completes after 250 ms without contention
        if self.claim == ClaimState::Claiming && now.saturating_sub(self.claim_sent_us) > 250_000 {
            self.claim = ClaimState::Claimed;
            if let Some(a) = self.own_address {
                self.push_alert("info", "own_address", format!("Claimed address {a}"));
            }
            let req = N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0, 255, iso::iso_request(iso::PGN_ADDRESS_CLAIM));
            let _ = self.send(req, false);
            self.bump();
        }
        if self.claim == ClaimState::GatewayManaged || self.claim == ClaimState::Claimed {
            let pending_new: Vec<u8> = self.registry.devices.values().filter(|d| d.name.is_some() && !d.queried && d.online).filter_map(|d| d.address).collect();
            if self.can_query() {
                for a in pending_new {
                    self.enqueue_discovery(a);
                }
            }
        }
        // discovery pacing: up to 3 requests per tick
        if self.can_query() {
            for _ in 0..3 {
                let Some(mut d) = self.discovery.pop_front() else { break };
                let pgn_req = d.msg.pgn;
                // Skip if already answered.
                let answered = self.registry.by_address(d.dst).map(|dev| {
                    if pgn_req == iso::PGN_ISO_REQUEST {
                        match iso::parse_iso_request(&d.msg.data) {
                            Some(iso::PGN_PRODUCT_INFO) => dev.product.is_some(),
                            Some(iso::PGN_CONFIG_INFO) => dev.config.is_some(),
                            _ => false,
                        }
                    } else {
                        match d.msg.data.get(12) {
                            Some(0) => dev.tx_pgns.is_some(),
                            Some(1) => dev.rx_pgns.is_some(),
                            _ => false,
                        }
                    }
                });
                if answered == Some(true) {
                    continue;
                }
                let _ = self.send(d.msg.clone(), false);
                d.attempts += 1;
                if d.attempts < 3 {
                    self.discovery.push_back(d);
                }
            }
        }
        // heartbeat every 60 s
        if self.claim == ClaimState::Claimed && now.saturating_sub(self.last_hb_us) > 60_000_000 {
            self.last_hb_us = now;
            self.hb_seq = self.hb_seq.wrapping_add(1);
            let hb = iso::Heartbeat { interval_ms: Some(60_000), sequence: self.hb_seq }.encode();
            let _ = self.send(N2kMessage::new(7, iso::PGN_HEARTBEAT, 0, 255, hb), false);
        }
        if let Some(u) = self.armed_until {
            if u <= now {
                self.armed_until = None;
                self.push_alert("info", "tx_armed", "Transmit disarmed (timeout)".into());
            }
        }
        if let Some(c) = self.capture.as_mut() {
            c.maybe_flush();
        }
        self.session_tick(now);
    }
}

// -------------------------------------------------------------------- sessions

impl State {
    fn cap_frame(&mut self, unix: u64, f: &RawCanFrame) {
        if let Some(c) = self.capture.as_mut() {
            c.write_frame(unix, f);
        }
        if let Some(c) = self.session_cap.as_mut() {
            c.write_frame(unix, f);
        }
    }

    pub fn session_open_dir(&self) -> PathBuf {
        self.data_dir.join(crate::session::OPEN_DIR)
    }

    fn gateway_label(&self) -> Option<String> {
        self.iface.as_ref().map(|i| {
            let rest = i.uri.split_once(':').map(|(_, r)| r).unwrap_or("");
            let host = if let Some(h) = rest.strip_prefix("//").or_else(|| rest.split_once("://").map(|x| x.1)) {
                h.split(['/', '?']).next().unwrap_or("").to_string()
            } else if rest.starts_with("/dev") || rest.starts_with("COM") {
                rest.split('?').next().unwrap_or("").to_string()
            } else {
                String::new()
            };
            if host.is_empty() { i.name.clone() } else { format!("{} · {host}", i.name) }
        })
    }

    fn session_capture_open(&mut self) {
        let Some(sess) = self.session.as_mut() else { return };
        let dir = self.data_dir.join(crate::session::OPEN_DIR);
        let name = format!("capture-{:03}", sess.capture_files.len() + 1);
        let iface = self.iface.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| "n2k".into());
        match Capture::start(&dir, CaptureFormat::Pcapng, Some(name), &iface) {
            Ok(c) => {
                sess.capture_files.push(c.file_name());
                self.session_cap = Some(c);
            }
            Err(e) => self.push_alert("error", "session", format!("Cannot record the session: {e}")),
        }
    }

    fn session_capture_close(&mut self) {
        if let (Some(mut c), Some(s)) = (self.session_cap.take(), self.session.as_mut()) {
            c.finish();
            let i = c.info();
            s.frames += i.frames;
            s.messages += i.messages;
            s.capture_bytes += i.bytes;
        }
    }

    /// Start a new session (the current one, if any, must be ended first).
    pub fn session_begin(&mut self) {
        if self.session.is_some() {
            return;
        }
        let dir = self.session_open_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let mut s = crate::session::Session::new(self.technician.clone(), self.gateway_label());
        if let Some(gw) = &s.gateway {
            if let Some(e) = s.events.last_mut() {
                e.text = format!("Recording started on {gw}.");
            }
        }
        if let Some((lat, lon, src)) = self.last_fix {
            s.position = Some(crate::session::Position { lat, lon, source: self.registry.by_address(src).and_then(|d| d.model.clone()).unwrap_or_else(|| "GPS".into()) });
        }
        self.session = Some(s);
        if let Some(v) = self.ais.vessel() {
            self.session_set_vessel(v);
        }
        self.session_capture_open();
        self.session_last_save_us = 0;
        self.push_alert("info", "session", "Service session started".into());
        self.session_save();
    }

    /// Set the vessel identity and look up the previous visit.
    pub fn session_set_vessel(&mut self, v: crate::session::Vessel) {
        let root = self.sessions_dir.clone();
        let Some(s) = self.session.as_mut() else { return };
        let text = match (&v.name, v.mmsi) {
            (Some(n), Some(m)) => format!("Boat recognised from its AIS transponder: {n}, MMSI {m}."),
            (None, Some(m)) => format!("Boat recognised from its AIS transponder: MMSI {m}."),
            (Some(n), None) => format!("Boat set to {n}."),
            _ => "Boat details updated.".into(),
        };
        let changed_id = s.vessel.id() != v.id();
        let renamed = s.vessel.name != v.name && v.name.is_some();
        s.vessel = v;
        if changed_id {
            s.event("vessel", text);
        } else if renamed {
            s.event("vessel", format!("Boat name set to {}.", s.vessel.display_name()));
        }
        if changed_id {
            s.previous = None;
            s.visit_number = 1;
            if let Some(id) = s.vessel.id() {
                if let Some(h) = crate::session::find_vessel(&root, &id) {
                    s.visit_number = h.sessions.len() as u32 + 1;
                    if let Some(p) = h.sessions.first() {
                        s.previous = Some(crate::session::PrevVisit { dir: format!("{}/{}", h.folder, p.dir), started_ms: p.started_ms, ended_ms: p.ended_ms });
                    }
                }
            }
        }
    }

    /// Reload a session left open when Keelwire was closed.
    pub fn session_reopen(&mut self) {
        let p = self.session_open_dir().join("session.json");
        let Ok(t) = std::fs::read_to_string(&p) else { return };
        let Ok(mut s) = serde_json::from_str::<crate::session::Session>(&t) else { return };
        if s.ended_ms.is_some() {
            return;
        }
        // Bytes written before the restart.
        s.capture_bytes = s.capture_files.iter().filter_map(|f| std::fs::metadata(self.session_open_dir().join(f)).ok()).map(|m| m.len()).sum();
        if s.paused.is_none() {
            s.pause("closed");
        }
        s.ask_resume = true;
        self.session = Some(s);
    }

    pub fn session_save(&mut self) {
        let dir = self.session_open_dir();
        if let Some(s) = &self.session {
            let tmp = dir.join("session.json.tmp");
            if std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("session.json"));
            }
        }
        self.session_last_save_us = clock::now_us();
    }

    fn session_on_connect(&mut self) {
        let is_replay = self.iface.as_ref().map(|i| i.uri.starts_with("replay")).unwrap_or(false);
        match self.session.as_mut() {
            None if self.auto_sessions && !is_replay && self.offline.is_none() => self.session_begin(),
            Some(s) if s.is_paused() => {
                let gap = s.paused.as_ref().map(|p| crate::session::now_ms().saturating_sub(p.from_ms)).unwrap_or(0);
                if !s.ask_resume && s.paused.as_ref().map(|p| p.reason == "disconnected" || p.reason == "background").unwrap_or(false) && gap < crate::session::RESUME_WINDOW_MS {
                    s.resume();
                    self.session_capture_open();
                    self.session_save();
                } else {
                    s.ask_resume = true;
                }
            }
            _ => {}
        }
    }

    fn session_on_disconnect(&mut self) {
        if let Some(s) = self.session.as_mut() {
            if !s.is_paused() {
                s.pause("disconnected");
                self.session_capture_close();
                self.session_save();
            }
        }
    }

    pub fn session_pause_user(&mut self) {
        if let Some(s) = self.session.as_mut() {
            if !s.is_paused() {
                s.pause("user");
                self.session_capture_close();
                self.session_save();
            }
        }
    }

    pub fn session_resume_user(&mut self) {
        if let Some(s) = self.session.as_mut() {
            if s.is_paused() {
                s.resume();
                self.session_capture_open();
                self.session_save();
            }
        }
    }

    fn session_observe(&mut self, m: &N2kMessage) {
        match m.pgn {
            129025 | 129029 => {
                let dec = self.db.decode(m.pgn, &m.data);
                if let (Some(lat), Some(lon)) = (dec.get("latitude").and_then(|v| v.as_f64()), dec.get("longitude").and_then(|v| v.as_f64())) {
                    if lat.abs() <= 90.0 && lon.abs() <= 180.0 && !(lat == 0.0 && lon == 0.0) {
                        self.last_fix = Some((lat, lon, m.source));
                        if let Some(s) = self.session.as_mut().filter(|s| s.position.is_none() && !s.is_paused()) {
                            let src = self.registry.by_address(m.source).and_then(|d| d.model.clone()).unwrap_or_else(|| "GPS".into());
                            s.position = Some(crate::session::Position { lat, lon, source: src });
                        }
                    }
                }
            }
            129038 | 129039 | 129794 | 129809 | 129810 => {
                let db = self.db.clone();
                self.ais.observe(&db, m);
                if let Some(v) = self.ais.vessel() {
                    let set = self.session.as_ref().map(|s| s.vessel.source == "none" || (s.vessel.source == "ais" && !s.vessel.confirmed && (s.vessel.name != v.name || s.vessel.callsign != v.callsign))).unwrap_or(false);
                    if set {
                        let first = self.session.as_ref().map(|s| s.vessel.source == "none").unwrap_or(false);
                        if first {
                            self.session_set_vessel(v);
                        } else if let Some(s) = self.session.as_mut() {
                            // Static data arrived after the first position report.
                            if s.vessel.name.is_none() {
                                if let (Some(n), Some(e)) = (&v.name, s.events.iter_mut().rev().find(|e| e.kind == "vessel")) {
                                    e.text = format!("Boat recognised from its AIS transponder: {n}, MMSI {}.", v.mmsi.unwrap_or(0));
                                }
                            }
                            s.vessel.name = v.name;
                            s.vessel.callsign = v.callsign;
                            s.vessel.ship_type = v.ship_type;
                            s.vessel.length_m = v.length_m;
                            s.vessel.beam_m = v.beam_m;
                            s.vessel.ais_class = v.ais_class;
                        }
                        self.bump();
                    }
                }
            }
            _ => {}
        }
    }

    fn session_tick(&mut self, now: u64) {
        if self.session.is_none() {
            return;
        }
        self.session_tick += 1;
        let paused = self.session.as_ref().map(|s| s.is_paused()).unwrap_or(true);
        if !paused && self.session_tick.is_multiple_of(10) {
            if let Some(l) = self.metrics.last_second() {
                let s = self.session.as_mut().unwrap();
                s.load.sum += l.load_pct;
                s.load.n += 1;
                if l.load_pct > s.load.peak {
                    s.load.peak = l.load_pct;
                    s.load.peak_ms = crate::session::now_ms();
                }
            }
        }
        if !paused && self.session_tick.is_multiple_of(100) {
            let devs = self.registry.list();
            let db = &self.db;
            let findings = keelwire_core::analysis::analyze(&devs, &|p| db.name(p).unwrap_or("PGN").to_string());
            let snaps: Vec<crate::session::DeviceSnap> = devs.iter().map(|d| crate::session::DeviceSnap::from_device(d)).collect();
            let others = self.ais.others();
            let incomplete = self.metrics.fp_timeouts + self.metrics.fp_orphans;
            let errors = if self.caps.error_frames { Some(self.metrics.total_errors) } else { None };
            let s = self.session.as_mut().unwrap();
            let before: Vec<String> = s.findings.iter().map(|f| f.sig.clone()).collect();
            s.note_findings(&findings);
            let fresh: Vec<String> = s.findings.iter().filter(|f| !before.contains(&f.sig)).map(|f| f.message.clone()).collect();
            // Findings already there at the start are reported in the first event.
            let started = s.events.iter().any(|e| e.kind == "finding" || e.kind == "survey");
            if !started && crate::session::now_ms().saturating_sub(s.started_ms) > 20_000 && !snaps.is_empty() {
                s.devices_start = snaps;
                let n = s.devices_start.len();
                let p = s.findings.len();
                s.event("survey", format!("{n} devices on the bus, {} found.", match p { 0 => "no problems".to_string(), 1 => "1 problem".to_string(), k => format!("{k} problems") }));
            } else if started {
                for m in fresh {
                    s.event("finding", format!("New problem: {m}"));
                }
            }
            s.other_ships = s.other_ships.max(others);
            s.incomplete = incomplete;
            s.bus_errors = errors;
            if s.devices_start.is_empty() {
                // nothing yet
            } else if !self.session_open_dir().join("boat-before.keelwire").exists() {
                let b = crate::boatfile::BoatFile::from_state(self, self.session.as_ref().and_then(|s| s.vessel.name.clone()));
                let _ = std::fs::write(self.session_open_dir().join("boat-before.keelwire"), serde_json::to_vec_pretty(&b).unwrap_or_default());
            }
        }
        if now.saturating_sub(self.session_last_save_us) > 30_000_000 {
            if let Some(c) = self.session_cap.as_mut() {
                c.maybe_flush();
            }
            self.session_save();
        }
    }
}

fn own_product() -> ProductInfo {
    ProductInfo {
        nmea2000_version: Some(2100),
        product_code: Some(0),
        model_id: "Keelwire".into(),
        software_version: crate::VERSION.into(),
        model_version: std::env::consts::OS.into(),
        serial_code: format!("{:06}", host_unique() % 1_000_000),
        certification_level: Some(0),
        load_equivalency: Some(0),
    }
}

async fn ingest(engine: Arc<Engine>, mut rx: mpsc::Receiver<BusEvent>) {
    let mut batch = Vec::with_capacity(256);
    while let Some(ev) = rx.recv().await {
        batch.push(ev);
        while batch.len() < 256 {
            match rx.try_recv() {
                Ok(e) => batch.push(e),
                Err(_) => break,
            }
        }
        let mut st = engine.state.lock();
        for ev in batch.drain(..) {
            st.on_event(ev);
        }
    }
}

async fn ticker(engine: Arc<Engine>) {
    let mut iv = tokio::time::interval(Duration::from_millis(100));
    loop {
        iv.tick().await;
        engine.state.lock().tick();
    }
}

/// Helper for API: parse a hex string message for raw TX.
pub fn message_from_parts(prio: u8, pgn: u32, dst: u8, data: Vec<u8>) -> N2kMessage {
    N2kMessage::new(prio, pgn, 0, dst, data)
}

#[allow(dead_code)]
pub type DeviceMap = HashMap<String, Device>;

// -------------------------------------------------------------------- session API

#[derive(Deserialize, Default)]
pub struct SessionPatch {
    pub name: Option<String>,
    pub hin: Option<String>,
    pub technician: Option<String>,
    /// Accept the identity read from AIS.
    pub confirm: Option<bool>,
    /// Ignore AIS and use typed details (or none).
    pub manual: Option<bool>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct EndOptions {
    #[serde(default = "yes")]
    pub owner_pdf: bool,
    /// "pdf", "html" or null for none
    pub technician: Option<String>,
    #[serde(default = "yes")]
    pub bundle: bool,
    #[serde(default)]
    pub strip_other_ais: bool,
}

fn yes() -> bool {
    true
}

impl Default for EndOptions {
    fn default() -> Self {
        EndOptions { owner_pdf: true, technician: Some("pdf".into()), bundle: true, strip_other_ais: false }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct EndResult {
    pub dir: String,
    pub rel: String,
    pub files: Vec<String>,
}

impl Engine {
    pub fn session_status(&self) -> serde_json::Value {
        let st = self.state.lock();
        let now = crate::session::now_ms();
        let live = st.session_cap.as_ref().map(|c| c.info());
        let own = st.ais.own().cloned();
        let others = st.ais.others();
        match &st.session {
            None => serde_json::json!({
                "state": "none", "now_ms": now, "technician": st.technician, "ais_own": own, "other_ships": others,
                "sessions_dir": st.sessions_dir, "auto": st.auto_sessions,
            }),
            Some(s) => {
                let state = if s.is_paused() { "paused" } else { "recording" };
                serde_json::json!({
                    "state": state,
                    "reason": s.paused.as_ref().map(|p| p.reason.clone()),
                    "now_ms": now,
                    "active_ms": s.active_ms(now),
                    "bytes": s.capture_bytes + live.as_ref().map(|i| i.bytes).unwrap_or(0),
                    "frames": s.frames + live.as_ref().map(|i| i.frames).unwrap_or(0),
                    "session": s,
                    "changes_ok": s.ok_changes().count(),
                    "fixed": s.fixed().len(),
                    "open": s.open().len(),
                    "technician": st.technician,
                    "ais_own": own,
                    "other_ships": others,
                    "sessions_dir": st.sessions_dir,
                    "auto": st.auto_sessions,
                })
            }
        }
    }

    pub fn session_start(&self) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if st.session.is_some() {
            return Err(EngineError::Bad("a session is already open; end it first".into()));
        }
        st.session_begin();
        let connected = matches!(st.iface_status, InterfaceStatus::Connected { .. });
        if !connected {
            if let Some(s) = st.session.as_mut() {
                s.pause("disconnected");
            }
            st.session_capture_close();
            st.session_save();
        }
        st.bump();
        Ok(())
    }

    pub fn session_pause(&self) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if st.session.is_none() {
            return Err(EngineError::Bad("no session".into()));
        }
        st.session_pause_user();
        Ok(())
    }

    /// The app left the screen (phones and tablets: iOS suspends it, so the
    /// gateway is no longer heard) or came back.
    pub fn app_background(&self, background: bool) {
        let mut st = self.state.lock();
        let connected = matches!(st.iface_status, InterfaceStatus::Connected { .. });
        let Some(s) = st.session.as_mut() else { return };
        if background {
            if s.is_paused() {
                return;
            }
            s.pause("background");
            st.session_capture_close();
            st.session_save();
        } else if connected && !s.ask_resume && s.paused.as_ref().map(|p| p.reason == "background").unwrap_or(false) {
            s.resume();
            st.session_capture_open();
            st.session_save();
        }
        // Not connected: the gateway driver reconnects and the session resumes then.
    }

    /// A session is open and recording (the app keeps the screen awake).
    pub fn session_recording(&self) -> bool {
        self.state.lock().session.as_ref().map(|s| !s.is_paused()).unwrap_or(false)
    }

    pub fn session_resume(&self) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if st.session.is_none() {
            return Err(EngineError::Bad("no session".into()));
        }
        if !matches!(st.iface_status, InterfaceStatus::Connected { .. }) {
            return Err(EngineError::Bad("connect to a gateway first; recording resumes when it connects".into()));
        }
        st.session_resume_user();
        Ok(())
    }

    pub fn session_update(&self, p: SessionPatch) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        if let Some(t) = &p.technician {
            let t = t.trim().to_string();
            st.technician = Some(t.clone()).filter(|t| !t.is_empty());
            let tech = st.technician.clone();
            if let Some(s) = st.session.as_mut() {
                s.technician = tech;
            }
            drop(st);
            self.save_setting("technician", serde_json::json!(t))?;
            st = self.state.lock();
        }
        if st.session.is_none() {
            return if p.name.is_some() || p.hin.is_some() || p.confirm.is_some() || p.manual.is_some() { Err(EngineError::Bad("no session".into())) } else { Ok(()) };
        }
        let mut v = st.session.as_ref().unwrap().vessel.clone();
        if p.manual == Some(true) {
            v = crate::session::Vessel { source: "manual".into(), confirmed: true, ..Default::default() };
        }
        if p.confirm == Some(true) {
            if let Some(a) = st.ais.vessel() {
                if v.source != "ais" || v.mmsi != a.mmsi {
                    v = a;
                }
            }
            v.confirmed = true;
        }
        if let Some(n) = p.name {
            let n = n.trim().to_string();
            v.name = Some(n).filter(|n| !n.is_empty());
            if v.source == "none" {
                v.source = "manual".into();
            }
            v.confirmed = true;
        }
        if let Some(h) = p.hin {
            let h = h.trim().to_uppercase();
            v.hin = Some(h).filter(|h| !h.is_empty());
            if v.source == "none" {
                v.source = "manual".into();
            }
            v.confirmed = true;
        }
        st.session_set_vessel(v);
        st.session_save();
        st.bump();
        Ok(())
    }

    pub fn session_note(&self, text: &str) -> Result<(), EngineError> {
        let t = text.trim();
        if t.is_empty() {
            return Err(EngineError::Bad("empty note".into()));
        }
        let mut st = self.state.lock();
        let s = st.session.as_mut().ok_or_else(|| EngineError::Bad("no session".into()))?;
        s.notes.push(crate::session::Note { ts_ms: crate::session::now_ms(), text: t.chars().take(4000).collect() });
        st.session_save();
        Ok(())
    }

    pub fn session_delete_note(&self, ts_ms: u64) -> Result<(), EngineError> {
        let mut st = self.state.lock();
        let s = st.session.as_mut().ok_or_else(|| EngineError::Bad("no session".into()))?;
        s.notes.retain(|n| n.ts_ms != ts_ms);
        st.session_save();
        Ok(())
    }

    pub fn session_record_changes(&self, changes: Vec<crate::session::Change>) {
        if changes.is_empty() {
            return;
        }
        let mut st = self.state.lock();
        if let Some(s) = st.session.as_mut() {
            s.changes.extend(changes);
            st.session_save();
        }
    }

    /// What changed on this boat since its last saved session.
    pub fn since_last(&self) -> serde_json::Value {
        let (prev, now_devs, root) = {
            let st = self.state.lock();
            let Some(s) = &st.session else { return serde_json::json!(null) };
            let Some(p) = s.previous.clone() else { return serde_json::json!(null) };
            // What we found on arrival (once surveyed), not what we have changed since.
            let devs: Vec<crate::session::DeviceSnap> = if s.devices_start.is_empty() { st.registry.list().into_iter().map(crate::session::DeviceSnap::from_device).collect() } else { s.devices_start.clone() };
            (p, devs, st.sessions_dir.clone())
        };
        let Ok(t) = std::fs::read_to_string(root.join(&prev.dir).join("session.json")) else { return serde_json::json!(null) };
        let Ok(old) = serde_json::from_str::<crate::session::Session>(&t) else { return serde_json::json!(null) };
        let base = if old.devices_end.is_empty() { &old.devices_start } else { &old.devices_end };
        let items = crate::session::diff_devices(base, &now_devs);
        serde_json::json!({ "previous": prev, "items": items })
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.state.lock().sessions_dir.clone()
    }

    fn save_setting(&self, key: &str, value: serde_json::Value) -> Result<(), EngineError> {
        let path = self.settings_path();
        let mut v: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
        v[key] = value;
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
        std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default()).map_err(|e| EngineError::Bad(format!("cannot save settings: {e}")))
    }

    /// End the session: write reports and the bundle into the boat's folder.
    pub async fn session_end(self: &Arc<Self>, opts: EndOptions) -> Result<EndResult, EngineError> {
        let ctx = {
            let mut st = self.state.lock();
            if st.session.is_none() {
                return Err(EngineError::Bad("no session".into()));
            }
            st.session_capture_close();
            let devs: Vec<Device> = st.registry.list().into_iter().cloned().collect();
            let db = st.db.clone();
            let findings = keelwire_core::analysis::analyze(&devs.iter().collect::<Vec<_>>(), &|p| db.name(p).unwrap_or("PGN").to_string());
            let boat_after = crate::boatfile::BoatFile::from_state(&st, st.session.as_ref().and_then(|s| s.vessel.name.clone()));
            let traffic = st.traffic.snapshot();
            let others = st.ais.others();
            let open_dir = st.session_open_dir();
            let root = st.sessions_dir.clone();
            let mut s = st.session.take().unwrap();
            let now = crate::session::now_ms();
            if let Some(mut p) = s.paused.take() {
                p.to_ms = Some(now);
                s.pauses.push(p);
            }
            s.ended_ms = Some(now);
            s.note_findings(&findings);
            s.devices_end = devs.iter().map(crate::session::DeviceSnap::from_device).collect();
            if s.devices_start.is_empty() {
                s.devices_start = s.devices_end.clone();
            }
            s.other_ships = s.other_ships.max(others);
            s.event("end", "Session ended.".into());
            st.push_alert("info", "session", "Service session ended".into());
            st.bump();
            crate::report::EndCtx { session: s, boat_after, traffic, devices: devs, open_dir, root, db }
        };
        let r = tokio::task::spawn_blocking(move || crate::report::finalize(ctx, &opts)).await.map_err(|e| EngineError::Bad(e.to_string()))?;
        r.map_err(EngineError::Bad)
    }

    /// End the open session with default outputs and start a new one.
    pub async fn session_new(self: &Arc<Self>) -> Result<(), EngineError> {
        if self.state.lock().session.is_some() {
            self.session_end(EndOptions { owner_pdf: false, technician: Some("html".into()), bundle: true, strip_other_ais: false }).await?;
        }
        let mut st = self.state.lock();
        st.session_begin();
        if !matches!(st.iface_status, InterfaceStatus::Connected { .. }) {
            st.session_on_disconnect();
        }
        Ok(())
    }
}
