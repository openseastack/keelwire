//! Service sessions (SPEC §11.5): one visit to one boat. Starts when a
//! gateway connects, pauses on disconnect, records the whole bus to pcapng,
//! logs every change sent, keeps notes, and on End writes reports and a
//! bundle into the boat's history folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use keelwire_core::analysis::{Finding, Severity};
use keelwire_core::canboat::PgnDb;
use keelwire_core::registry::Device;
use keelwire_core::N2kMessage;
use serde::{Deserialize, Serialize};

pub const OPEN_DIR: &str = "session-open";
/// A gap longer than this asks "continue or start new" instead of resuming.
pub const RESUME_WINDOW_MS: u64 = 30 * 60 * 1000;

pub fn now_ms() -> u64 {
    keelwire_hal::clock::unix_now_us() / 1000
}

// ------------------------------------------------------------------ vessel

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Vessel {
    pub mmsi: Option<u32>,
    pub name: Option<String>,
    pub callsign: Option<String>,
    pub hin: Option<String>,
    pub ship_type: Option<String>,
    pub length_m: Option<f64>,
    pub beam_m: Option<f64>,
    /// "A" or "B"
    pub ais_class: Option<String>,
    /// "ais", "manual" or "none"
    pub source: String,
    /// The technician confirmed (or typed) the identity.
    pub confirmed: bool,
}

impl Vessel {
    pub fn display_name(&self) -> String {
        self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Unnamed boat".into())
    }

    /// History key: MMSI, else HIN, else name.
    pub fn id(&self) -> Option<String> {
        if let Some(m) = self.mmsi {
            return Some(format!("mmsi-{m}"));
        }
        if let Some(h) = self.hin.as_ref().filter(|h| !h.trim().is_empty()) {
            return Some(format!("hin-{}", h.trim().to_uppercase()));
        }
        self.name.as_ref().filter(|n| !n.trim().is_empty()).map(|n| format!("name-{}", n.trim().to_lowercase()))
    }

    pub fn folder_name(&self) -> String {
        let name = self.name.as_ref().filter(|n| !n.trim().is_empty()).map(|n| title_case(n.trim()));
        let id = self.mmsi.map(|m| m.to_string()).or_else(|| self.hin.clone().filter(|h| !h.trim().is_empty()).map(|h| h.trim().to_uppercase()));
        let raw = match (name, id) {
            (Some(n), Some(i)) => format!("{n} ({i})"),
            (Some(n), None) => n,
            (None, Some(i)) => i,
            (None, None) => "Unnamed boats".into(),
        };
        safe_folder(&raw)
    }
}

pub fn title_case(s: &str) -> String {
    if s.chars().any(|c| c.is_lowercase()) {
        return s.to_string();
    }
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn safe_folder(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || " -_().,'".contains(c) { c } else { '_' }).collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() {
        "boat".into()
    } else {
        t.chars().take(80).collect()
    }
}

// ------------------------------------------------------------------ AIS

pub const AIS_PGNS: [u32; 16] = [129038, 129039, 129040, 129041, 129793, 129794, 129798, 129801, 129802, 129803, 129804, 129805, 129806, 129807, 129809, 129810];

#[derive(Clone, Debug, Default, Serialize)]
pub struct AisTarget {
    pub mmsi: u32,
    pub name: Option<String>,
    pub callsign: Option<String>,
    pub ship_type: Option<String>,
    pub length_m: Option<f64>,
    pub beam_m: Option<f64>,
    pub class: Option<String>,
    /// Reports flagged as this transceiver's own transmissions.
    pub own_reports: u32,
    pub last_ms: u64,
    /// Bus address of the transponder that reported it.
    pub source: u8,
}

#[derive(Default)]
pub struct AisTracker {
    pub targets: BTreeMap<u32, AisTarget>,
}

fn clean_text(v: Option<&keelwire_core::Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).map(|s| s.trim_end_matches(['@', ' ', '\0']).trim().to_string()).filter(|s| !s.is_empty())
}

impl AisTracker {
    /// Returns true when the own vessel's identity (name, call sign...) changed.
    pub fn observe(&mut self, db: &PgnDb, m: &N2kMessage) -> bool {
        if !matches!(m.pgn, 129038 | 129039 | 129794 | 129809 | 129810) {
            return false;
        }
        let d = db.decode(m.pgn, &m.data);
        let Some(mmsi) = d.get("userId").and_then(|v| v.as_f64()).map(|v| v as u32).filter(|v| *v > 0 && *v < 1_000_000_000) else { return false };
        let own = d.get("aisTransceiverInformation").and_then(|v| v.as_f64()).map(|v| (2.0..=4.0).contains(&v)).unwrap_or(false);
        let t = self.targets.entry(mmsi).or_insert_with(|| AisTarget { mmsi, ..Default::default() });
        let before = (t.name.clone(), t.callsign.clone(), t.own_reports > 0);
        t.last_ms = now_ms();
        t.source = m.source;
        if own {
            t.own_reports = t.own_reports.saturating_add(1);
        }
        match m.pgn {
            129038 | 129794 => t.class = Some("A".into()),
            129039 | 129809 | 129810 => t.class = Some("B".into()),
            _ => {}
        }
        if let Some(n) = clean_text(d.get("name")) {
            t.name = Some(n);
        }
        if let Some(c) = clean_text(d.get("callsign")) {
            t.callsign = Some(c);
        }
        if let Some(f) = d.fields.iter().find(|f| f.id == "typeOfShip") {
            t.ship_type = f.text.clone().or_else(|| f.value.as_f64().map(|v| format!("type {v}")));
        }
        if let Some(l) = d.get("length").and_then(|v| v.as_f64()).filter(|v| *v > 0.0) {
            t.length_m = Some(l);
        }
        if let Some(b) = d.get("beam").and_then(|v| v.as_f64()).filter(|v| *v > 0.0) {
            t.beam_m = Some(b);
        }
        if self.targets.len() > 2000 {
            let oldest = self.targets.values().filter(|t| t.own_reports == 0).min_by_key(|t| t.last_ms).map(|t| t.mmsi);
            if let Some(o) = oldest {
                self.targets.remove(&o);
            }
        }
        let t = &self.targets[&mmsi];
        t.own_reports > 0 && (t.name.clone(), t.callsign.clone(), true) != before
    }

    /// This boat's own transponder: the target with the most own reports.
    pub fn own(&self) -> Option<&AisTarget> {
        self.targets.values().filter(|t| t.own_reports > 0).max_by_key(|t| (t.own_reports, t.last_ms))
    }

    pub fn others(&self) -> usize {
        self.targets.values().filter(|t| t.own_reports == 0).count()
    }

    pub fn vessel(&self) -> Option<Vessel> {
        self.own().map(|t| Vessel {
            mmsi: Some(t.mmsi),
            name: t.name.clone(),
            callsign: t.callsign.clone(),
            hin: None,
            ship_type: t.ship_type.clone(),
            length_m: t.length_m,
            beam_m: t.beam_m,
            ais_class: t.class.clone(),
            source: "ais".into(),
            confirmed: false,
        })
    }
}

// ------------------------------------------------------------------ snapshots

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DeviceSnap {
    pub key: String,
    pub address: Option<u8>,
    pub manufacturer: Option<String>,
    pub mfr_code: Option<u16>,
    pub unique: Option<u32>,
    pub model: Option<String>,
    pub function: Option<String>,
    pub serial: Option<String>,
    pub software: Option<String>,
    pub device_instance: Option<u8>,
    pub system_instance: Option<u8>,
    pub install1: Option<String>,
    pub install2: Option<String>,
    pub len: Option<u8>,
    pub online: bool,
    /// PGN → primary-key signatures it sends (instances, sources).
    #[serde(default)]
    pub instances: BTreeMap<u32, Vec<String>>,
}

impl DeviceSnap {
    pub fn from_device(d: &Device) -> DeviceSnap {
        DeviceSnap {
            key: d.key.clone(),
            address: d.address,
            manufacturer: d.manufacturer.clone(),
            mfr_code: d.name.map(|n| n.manufacturer_code),
            unique: d.name.map(|n| n.unique_number),
            model: d.model.clone().or_else(|| d.product.as_ref().map(|p| p.model_id.clone())).filter(|s| !s.is_empty()),
            function: d.device_function_text.clone(),
            serial: d.product.as_ref().map(|p| p.serial_code.trim().to_string()).filter(|s| !s.is_empty()),
            software: d.product.as_ref().map(|p| p.software_version.trim().to_string()).filter(|s| !s.is_empty()),
            device_instance: d.device_instance(),
            system_instance: d.name.map(|n| n.system_instance),
            install1: d.config.as_ref().map(|c| c.installation_description1.clone()),
            install2: d.config.as_ref().map(|c| c.installation_description2.clone()),
            len: d.product.as_ref().and_then(|p| p.load_equivalency),
            online: d.online,
            instances: d.observed.iter().filter(|(p, o)| **p >= 127000 && !AIS_PGNS.contains(p) && !o.keys.is_empty()).map(|(p, o)| (*p, o.keys.clone())).collect(),
        }
    }

    pub fn label(&self) -> String {
        let m = self.model.clone().or_else(|| self.function.clone()).unwrap_or_else(|| "Device".into());
        match self.address {
            Some(a) => format!("{m} @{a}"),
            None => m,
        }
    }

    /// Physical identity, stable across instance (NAME) changes.
    pub fn ident(&self) -> String {
        match (&self.serial, self.mfr_code, self.unique) {
            (Some(s), Some(m), _) => format!("{m}:{}:{s}", self.model.clone().unwrap_or_default()),
            (_, Some(m), Some(u)) => format!("{m}:u{u}"),
            _ => self.key.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FindingSnap {
    pub sig: String,
    pub severity: String,
    pub message: String,
    pub suggestion: Option<String>,
    pub devices: Vec<String>,
    pub pgn: Option<u32>,
    pub first_ms: u64,
    pub last_ms: u64,
}

pub fn finding_sig(f: &Finding) -> String {
    let mut d = f.devices.clone();
    d.sort();
    format!("{}|{}|{}", f.kind, f.pgn.unwrap_or(0), d.join(","))
}

// ------------------------------------------------------------------ session

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pause {
    pub from_ms: u64,
    pub to_ms: Option<u64>,
    /// "disconnected", "user", "closed"
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Change {
    pub ts_ms: u64,
    pub device_key: Option<String>,
    pub device: String,
    pub serial: Option<String>,
    pub setting: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub method: String,
    /// "confirmed", "sent" (accepted, value not read back) or "failed"
    pub result: String,
    pub error: Option<String>,
    /// "setting", "control" or "raw"
    pub kind: String,
    /// Plain-language sentence for the owner report.
    pub plain: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub ts_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub ts_ms: u64,
    /// "start", "pause", "resume", "offline", "online", "vessel", "finding"
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoadStats {
    pub sum: f64,
    pub n: u64,
    pub peak: f64,
    pub peak_ms: u64,
}

impl LoadStats {
    pub fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub technician: Option<String>,
    pub vessel: Vessel,
    pub position: Option<Position>,
    pub gateway: Option<String>,
    pub paused: Option<Pause>,
    pub pauses: Vec<Pause>,
    /// Asked on reconnect after a long gap or a restart: continue or start new?
    #[serde(default)]
    pub ask_resume: bool,
    pub changes: Vec<Change>,
    pub notes: Vec<Note>,
    pub events: Vec<Event>,
    pub devices_start: Vec<DeviceSnap>,
    #[serde(default)]
    pub devices_end: Vec<DeviceSnap>,
    /// Every non-info finding seen during the session.
    pub findings: Vec<FindingSnap>,
    /// Signatures of findings present at the end.
    #[serde(default)]
    pub findings_open: Vec<String>,
    pub capture_files: Vec<String>,
    pub frames: u64,
    pub messages: u64,
    pub capture_bytes: u64,
    pub load: LoadStats,
    #[serde(default)]
    pub other_ships: usize,
    #[serde(default)]
    pub incomplete: u64,
    #[serde(default)]
    pub bus_errors: Option<u64>,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub visit_number: u32,
    #[serde(default)]
    pub previous: Option<PrevVisit>,
    #[serde(default)]
    pub since_last: Vec<DiffItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PrevVisit {
    pub dir: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

impl Session {
    pub fn new(technician: Option<String>, gateway: Option<String>) -> Session {
        let started = now_ms();
        let mut s = Session {
            id: local_fmt(started, "%Y-%m-%d-%H%M"),
            started_ms: started,
            technician,
            gateway,
            vessel: Vessel { source: "none".into(), ..Default::default() },
            app_version: crate::VERSION.into(),
            ..Default::default()
        };
        s.event("start", "Recording started.".into());
        s
    }

    pub fn event(&mut self, kind: &str, text: String) {
        self.events.push(Event { ts_ms: now_ms(), kind: kind.into(), text });
    }

    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    pub fn pause(&mut self, reason: &str) {
        if self.paused.is_some() {
            return;
        }
        let p = Pause { from_ms: now_ms(), to_ms: None, reason: reason.into() };
        self.event(
            "pause",
            match reason {
                "disconnected" => "Paused: gateway disconnected.".into(),
                "closed" => "Paused: Keelwire was closed.".into(),
                "background" => "Paused: app in the background.".into(),
                _ => "Paused by the technician.".into(),
            },
        );
        self.paused = Some(p);
    }

    pub fn resume(&mut self) {
        if let Some(mut p) = self.paused.take() {
            let now = now_ms();
            p.to_ms = Some(now);
            let mins = (now - p.from_ms) / 60_000;
            self.event("resume", format!("Resumed after {}.", if mins == 0 { "less than a minute".to_string() } else { format!("{mins} min") }));
            self.pauses.push(p);
        }
        self.ask_resume = false;
    }

    pub fn active_ms(&self, now: u64) -> u64 {
        let end = self.ended_ms.unwrap_or(now);
        let paused: u64 = self.pauses.iter().map(|p| p.to_ms.unwrap_or(end).saturating_sub(p.from_ms)).sum::<u64>() + self.paused.as_ref().map(|p| end.saturating_sub(p.from_ms)).unwrap_or(0);
        end.saturating_sub(self.started_ms).saturating_sub(paused)
    }

    pub fn ok_changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.result != "failed" && c.kind == "setting")
    }

    pub fn fixed(&self) -> Vec<&FindingSnap> {
        self.findings.iter().filter(|f| !self.findings_open.contains(&f.sig)).collect()
    }

    pub fn open(&self) -> Vec<&FindingSnap> {
        self.findings.iter().filter(|f| self.findings_open.contains(&f.sig)).collect()
    }

    /// Track findings: new ones are added, present ones refreshed.
    pub fn note_findings(&mut self, now: &[Finding]) {
        let t = now_ms();
        let mut sigs = Vec::new();
        for f in now.iter().filter(|f| f.severity != Severity::Info) {
            let sig = finding_sig(f);
            sigs.push(sig.clone());
            if let Some(x) = self.findings.iter_mut().find(|x| x.sig == sig) {
                x.last_ms = t;
            } else {
                self.findings.push(FindingSnap {
                    sig,
                    severity: if f.severity == Severity::Error { "error".into() } else { "warning".into() },
                    message: f.message.clone(),
                    suggestion: f.suggestion.clone(),
                    devices: f.devices.clone(),
                    pgn: f.pgn,
                    first_ms: t,
                    last_ms: t,
                });
            }
        }
        self.findings_open = sigs;
    }
}

// ------------------------------------------------------------------ time

pub fn local_fmt(ms: u64, fmt: &str) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => t.format(fmt).to_string(),
        None => String::new(),
    }
}

pub fn utc_offset_label(ms: u64) -> String {
    use chrono::{Offset, TimeZone};
    match chrono::Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => {
            let secs = t.offset().fix().local_minus_utc();
            let (sign, a) = if secs < 0 { ('−', -secs) } else { ('+', secs) };
            if a % 3600 == 0 {
                format!("UTC{sign}{}", a / 3600)
            } else {
                format!("UTC{sign}{}:{:02}", a / 3600, (a % 3600) / 60)
            }
        }
        None => "UTC".into(),
    }
}

pub fn fmt_duration(ms: u64) -> String {
    let m = ms / 60_000;
    if m < 60 {
        format!("{m} min")
    } else {
        format!("{} h {} min", m / 60, m % 60)
    }
}

pub fn fmt_lat_lon(lat: f64, lon: f64) -> String {
    let f = |v: f64, pos: char, neg: char, w: usize| {
        let a = v.abs();
        let d = a.floor();
        let m = (a - d) * 60.0;
        format!("{:0w$}°{:05.2}′{}", d as u32, m, if v >= 0.0 { pos } else { neg }, w = w)
    };
    format!("{} {}", f(lat, 'N', 'S', 2), f(lon, 'E', 'W', 3))
}

// ------------------------------------------------------------------ diff

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DiffItem {
    /// "added", "gone", "software", "setting"
    pub kind: String,
    pub text: String,
    /// Plain-language version for the owner.
    pub plain: String,
}

/// What changed between the end of the last visit and now.
/// `ours` lists settings our own previous session set, to flag those that
/// were changed back by someone else since.
pub fn diff_devices(prev: &[DeviceSnap], now: &[DeviceSnap]) -> Vec<DiffItem> {
    let mut out = Vec::new();
    let p: BTreeMap<String, &DeviceSnap> = prev.iter().map(|d| (d.ident(), d)).collect();
    let n: BTreeMap<String, &DeviceSnap> = now.iter().map(|d| (d.ident(), d)).collect();
    let serial = |d: &DeviceSnap| d.serial.as_ref().map(|s| format!(" · s/n {s}")).unwrap_or_default();
    for (k, d) in &n {
        if !p.contains_key(k) {
            out.push(DiffItem { kind: "added".into(), text: format!("{}{}", d.label(), serial(d)), plain: format!("New device: {}.", friendly(d)) });
        }
    }
    for (k, d) in &p {
        if !n.contains_key(k) {
            out.push(DiffItem { kind: "gone".into(), text: format!("{}{}", d.label(), serial(d)), plain: format!("No longer on the network: {}.", friendly(d)) });
        }
    }
    for (k, a) in &p {
        let Some(b) = n.get(k) else { continue };
        if a.software.is_some() && b.software.is_some() && a.software != b.software {
            out.push(DiffItem {
                kind: "software".into(),
                text: format!("{} {} → {}", b.label(), a.software.clone().unwrap_or_default(), b.software.clone().unwrap_or_default()),
                plain: format!("{} software updated to {}.", friendly(b), b.software.clone().unwrap_or_default()),
            });
        }
        let mut setting = |what: &str, x: String, y: String| {
            out.push(DiffItem { kind: "setting".into(), text: format!("{} {what} {x} → {y}", b.label()), plain: format!("{}: {} changed from {x} to {y}.", friendly(b), what.trim_start_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == 'P' || c == 'G' || c == 'N').trim()) });
        };
        if a.device_instance.is_some() && b.device_instance.is_some() && a.device_instance != b.device_instance {
            setting("device instance", a.device_instance.unwrap_or(0).to_string(), b.device_instance.unwrap_or(0).to_string());
        }
        if a.system_instance.is_some() && b.system_instance.is_some() && a.system_instance != b.system_instance {
            setting("system instance", a.system_instance.unwrap_or(0).to_string(), b.system_instance.unwrap_or(0).to_string());
        }
        for (what, x, y) in [("installation description 1", &a.install1, &b.install1), ("installation description 2", &a.install2, &b.install2)] {
            if x.is_some() && y.is_some() && x != y {
                setting(what, format!("“{}”", x.clone().unwrap_or_default()), format!("“{}”", y.clone().unwrap_or_default()));
            }
        }
        for (pgn, ka) in &a.instances {
            if let Some(kb) = b.instances.get(pgn) {
                let sa: BTreeSet<&String> = ka.iter().collect();
                let sb: BTreeSet<&String> = kb.iter().collect();
                if sa != sb {
                    // "Instance=1" → "Instance=0" reads better as "instance 1 → 0".
                    let split = |k: &Vec<String>| if k.len() == 1 { k[0].split_once('=').map(|(a, b)| (a.to_string(), b.to_string())) } else { None };
                    match (split(ka), split(kb)) {
                        (Some((fa, va)), Some((fb, vb))) if fa == fb && !fa.contains(',') => setting(&format!("PGN {pgn} data {}", fa.to_lowercase()), va, vb),
                        _ => setting(&format!("PGN {pgn}"), ka.join("; "), kb.join("; ")),
                    }
                }
            }
        }
    }
    out
}

pub fn friendly(d: &DeviceSnap) -> String {
    match (&d.function, &d.model) {
        (Some(f), Some(m)) => format!("{} ({m})", plain_function(f)),
        (None, Some(m)) => m.clone(),
        (Some(f), None) => plain_function(f),
        _ => "a device".into(),
    }
}

pub fn plain_function(f: &str) -> String {
    let l = f.to_lowercase();
    let s = if l.contains("fluid level") {
        "Tank sender"
    } else if l.contains("gnss") || l.contains("gps") || l.contains("position") {
        "GPS"
    } else if l.contains("heading") || l.contains("compass") || l.contains("attitude") {
        "Compass"
    } else if l.contains("depth") || l.contains("speed") && l.contains("water") {
        "Depth and speed"
    } else if l.contains("wind") || l.contains("weather") || l.contains("atmospheric") {
        "Weather station"
    } else if l.contains("engine") {
        "Engine interface"
    } else if l.contains("battery") || l.contains("dc ") || l.starts_with("dc") {
        "Battery monitor"
    } else if l.contains("ais") || l.contains("identification") {
        "AIS"
    } else if l.contains("switch") || l.contains("load controller") || l.contains("circuit") {
        "Switching"
    } else if l.contains("display") {
        "Display"
    } else if l.contains("gateway") || l.contains("bridge") {
        "Gateway"
    } else {
        return f.to_string();
    };
    s.to_string()
}

// ------------------------------------------------------------------ history

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VesselRecord {
    pub id: String,
    pub vessel: Vessel,
    pub folder: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub dir: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub technician: Option<String>,
    pub changes: usize,
    pub fixed: usize,
    pub open: usize,
    pub open_text: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VesselHistory {
    pub id: String,
    pub vessel: Vessel,
    pub folder: String,
    pub sessions: Vec<SessionSummary>,
}

pub fn summary_of(s: &Session, dir: &str, files: Vec<String>) -> SessionSummary {
    SessionSummary {
        id: s.id.clone(),
        dir: dir.into(),
        started_ms: s.started_ms,
        ended_ms: s.ended_ms,
        technician: s.technician.clone(),
        changes: s.ok_changes().count(),
        fixed: s.fixed().len(),
        open: s.open().len(),
        open_text: s.open().iter().map(|f| f.message.clone()).collect(),
        files,
    }
}

/// All boats in the history folder, newest session first.
pub fn list_history(root: &Path) -> Vec<VesselHistory> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(t) = std::fs::read_to_string(p.join("vessel.json")) else { continue };
        let Ok(rec) = serde_json::from_str::<VesselRecord>(&t) else { continue };
        let mut sessions: Vec<SessionSummary> = std::fs::read_dir(&p)
            .map(|rd| rd.flatten().filter_map(|s| std::fs::read_to_string(s.path().join("summary.json")).ok()).filter_map(|t| serde_json::from_str(&t).ok()).collect())
            .unwrap_or_default();
        sessions.sort_by_key(|s: &SessionSummary| std::cmp::Reverse(s.started_ms));
        out.push(VesselHistory { id: rec.id, vessel: rec.vessel, folder: rec.folder, sessions });
    }
    out.sort_by(|a, b| b.sessions.first().map(|s| s.started_ms).cmp(&a.sessions.first().map(|s| s.started_ms)));
    out
}

pub fn find_vessel(root: &Path, id: &str) -> Option<VesselHistory> {
    list_history(root).into_iter().find(|v| v.id == id)
}

/// The most recent saved session of a vessel (full record).
pub fn last_session(root: &Path, id: &str) -> Option<(Session, PathBuf)> {
    let v = find_vessel(root, id)?;
    let s = v.sessions.first()?;
    let dir = root.join(&v.folder).join(&s.dir);
    let t = std::fs::read_to_string(dir.join("session.json")).ok()?;
    serde_json::from_str(&t).ok().map(|x| (x, dir))
}

// ------------------------------------------------------------------ change log

/// Describe a request that changes a device, from the request and the device
/// as it was before and after. Returns None for requests that change nothing.
pub struct ChangeCtx<'a> {
    pub db: &'a PgnDb,
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a serde_json::Value,
    pub before: Option<&'a Device>,
    pub after: Option<&'a Device>,
    pub ok: bool,
    pub response: &'a serde_json::Value,
}

fn dev_label(d: Option<&Device>, key: &str) -> (String, Option<String>) {
    match d {
        Some(d) => {
            let s = DeviceSnap::from_device(d);
            (s.label(), s.serial)
        }
        None => (key.to_string(), None),
    }
}

fn fmt_num(v: f64, res: f64, unit: Option<&str>) -> String {
    let dec = if res >= 1.0 || res == 0.0 { 0 } else { (-res.log10()).ceil().clamp(0.0, 4.0) as usize };
    let s = format!("{v:.dec$}");
    let s = if s.starts_with('-') { s.replacen('-', "−", 1) } else { s };
    match unit {
        Some(u) if !u.is_empty() => format!("{s} {u}"),
        _ => s,
    }
}

/// Value of a field as text, from a decoded message.
fn field_text(db: &PgnDb, pgn: u32, data: &[u8], id: &str) -> Option<String> {
    let dec = db.decode(pgn, data);
    let f = dec.fields.iter().find(|f| f.id == id)?;
    if let Some(t) = &f.text {
        return Some(t.clone());
    }
    match &f.value {
        keelwire_core::Value::Text(t) => Some(format!("“{t}”")),
        keelwire_core::Value::Null => None,
        v => {
            let def = db.get(pgn).and_then(|d| d.iter().find(|d| !d.fallback)).and_then(|d| d.fields.iter().find(|x| x.id == id));
            let unit = def.and_then(|d| d.unit.clone());
            v.as_f64().map(|x| fmt_num(x, def.map(|d| d.resolution).unwrap_or(1.0), unit.as_deref()))
        }
    }
}

pub fn describe_change(c: &ChangeCtx) -> Vec<Change> {
    let parts: Vec<&str> = c.path.trim_start_matches("/api/v1/").split('/').collect();
    let key = if parts.first() == Some(&"devices") { parts.get(1).copied().unwrap_or("") } else { "" };
    let (label, serial) = dev_label(c.before.or(c.after), key);
    let snap = c.before.map(DeviceSnap::from_device);
    let friendly_dev = snap.as_ref().map(friendly).unwrap_or_else(|| label.clone());
    let err = if c.ok { None } else { c.response.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()) };
    let base = |setting: String, before: Option<String>, after: Option<String>, method: &str, result: &str, kind: &str, plain: Option<String>| Change {
        ts_ms: now_ms(),
        device_key: (!key.is_empty()).then(|| key.to_string()),
        device: label.clone(),
        serial: serial.clone(),
        setting,
        before,
        after,
        method: method.into(),
        result: if c.ok { result.into() } else { "failed".into() },
        error: err.clone(),
        kind: kind.into(),
        plain,
    };
    let b = c.body;
    let mut out = Vec::new();
    match (c.method, parts.as_slice()) {
        ("PATCH", ["devices", _, "standard"]) => {
            let after_snap = c.after.map(DeviceSnap::from_device);
            if let Some(v) = b.get("device_instance").and_then(|v| v.as_u64()) {
                let ok = after_snap.as_ref().and_then(|a| a.device_instance) == Some(v as u8);
                let before = snap.as_ref().and_then(|s| s.device_instance).map(|x| x.to_string());
                out.push(base("Device instance".into(), before.clone(), Some(v.to_string()), "126208 Command on 60928", if ok { "confirmed" } else { "sent" }, "setting", Some(format!("{friendly_dev}: device number changed from {} to {v}.", before.unwrap_or_else(|| "?".into())))));
            }
            if let Some(v) = b.get("system_instance").and_then(|v| v.as_u64()) {
                let ok = after_snap.as_ref().and_then(|a| a.system_instance) == Some(v as u8);
                out.push(base("System instance".into(), snap.as_ref().and_then(|s| s.system_instance).map(|x| x.to_string()), Some(v.to_string()), "126208 Command on 60928", if ok { "confirmed" } else { "sent" }, "setting", None));
            }
            for (f, name) in [("installation_description1", "Installation description 1"), ("installation_description2", "Installation description 2")] {
                if let Some(v) = b.get(f).and_then(|v| v.as_str()) {
                    let cur = |s: &Option<DeviceSnap>| s.as_ref().and_then(|s| if f.ends_with('1') { s.install1.clone() } else { s.install2.clone() });
                    let ok = cur(&after_snap).as_deref() == Some(v);
                    out.push(base(
                        format!("126998 {name}"),
                        Some(format!("“{}”", cur(&snap).unwrap_or_default())),
                        Some(format!("“{v}”")),
                        "126208 Write on 126998",
                        if ok { "confirmed" } else { "sent" },
                        "setting",
                        Some(format!("{friendly_dev} is now labelled “{v}” so it is easy to find on any display.")),
                    ));
                }
            }
        }
        ("POST", ["devices", _, "fields"]) => {
            let pgn = b.get("pgn").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let sel: Vec<(u8, f64)> = b.get("select").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| Some((x.get("field")?.as_u64()? as u8, x.get("value")?.as_f64()?))).collect()).unwrap_or_default();
            let vals: Vec<(u8, f64)> = b.get("values").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| Some((x.get("field")?.as_u64()? as u8, x.get("value")?.as_f64()?))).collect()).unwrap_or_default();
            let def = c.db.get(pgn).and_then(|d| d.iter().find(|d| !d.fallback));
            let pname = c.db.name(pgn).unwrap_or("PGN").to_string();
            // The occurrence being changed (e.g. instance 0 of a tank sender).
            let data: Option<Vec<u8>> = c.before.and_then(|d| d.observed.get(&pgn)).map(|o| {
                if sel.is_empty() || o.key_data.is_empty() {
                    return o.last_data.clone();
                }
                o.key_data
                    .iter()
                    .find(|kd| {
                        let dec = c.db.decode(pgn, kd);
                        sel.iter().all(|(ord, v)| {
                            def.and_then(|d| d.fields.iter().find(|f| f.order == *ord as usize)).and_then(|f| dec.get(&f.id)).and_then(|x| x.as_f64()).map(|x| (x - v).abs() < 0.5).unwrap_or(false)
                        })
                    })
                    .cloned()
                    .unwrap_or_else(|| o.last_data.clone())
            });
            let confirmed = c.response.get("confirmed").and_then(|v| v.as_bool()).unwrap_or(false);
            for (ord, v) in vals {
                let f = def.and_then(|d| d.fields.iter().find(|f| f.order == ord as usize));
                let fname = f.map(|f| f.name.clone()).unwrap_or_else(|| format!("field {ord}"));
                let before = match (f, &data) {
                    (Some(f), Some(d)) => field_text(c.db, pgn, d, &f.id),
                    _ => None,
                };
                let after = f.map(|f| match f.lookup.as_ref().and_then(|l| c.db.lookup_text(l, v as i64)) {
                    Some(t) => t.to_string(),
                    None => fmt_num(v, f.resolution, f.unit.as_deref()),
                });
                let plain = Some(format!("{friendly_dev}: {} changed from {} to {}.", fname.to_lowercase(), before.clone().unwrap_or_else(|| "?".into()), after.clone().unwrap_or_default()));
                out.push(base(format!("{pgn} {pname} · {fname}"), before, after, "126208 Command", if confirmed { "confirmed" } else { "sent" }, "setting", plain));
            }
        }
        ("POST", ["devices", _, "data-instance"]) => {
            let pgn = b.get("pgn").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let new = b.get("new").and_then(|v| v.as_u64()).unwrap_or(0);
            out.push(base(format!("{pgn} {} · Instance", c.db.name(pgn).unwrap_or("PGN")), None, Some(new.to_string()), "126208 Command", "sent", "setting", Some(format!("{friendly_dev}: data instance set to {new}."))));
        }
        ("POST", ["devices", _, "pgn-rate"]) => {
            let pgn = b.get("pgn").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let after = match b.get("interval_ms").and_then(|v| v.as_u64()) {
                None => "default".to_string(),
                Some(0) => "off".to_string(),
                Some(ms) => format!("{ms} ms"),
            };
            out.push(base(format!("{pgn} {} · Transmit interval", c.db.name(pgn).unwrap_or("PGN")), None, Some(after.clone()), "126208 Request", "sent", "setting", Some(format!("{friendly_dev}: how often it sends {} set to {after}.", c.db.name(pgn).unwrap_or("this data").to_lowercase()))));
        }
        ("POST", ["devices", _, "address"]) => {
            let a = b.get("address").and_then(|v| v.as_u64()).unwrap_or(0);
            out.push(base("Source address".into(), c.before.and_then(|d| d.address).map(|x| x.to_string()), Some(a.to_string()), "65240 Commanded Address", "sent", "setting", None));
        }
        ("POST", ["devices", _, "tank-capacity"]) => {
            let l = b.get("litres").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let before = c.before.and_then(|d| d.observed.get(&127505)).and_then(|o| field_text(c.db, 127505, &o.last_data, "capacity"));
            let confirmed = c.response.get("confirmed").and_then(|v| v.as_bool()).unwrap_or(false);
            out.push(base("127505 Fluid Level · Capacity".into(), before, Some(fmt_num(l, 0.1, Some("L"))), "126208 Command", if confirmed { "confirmed" } else { "sent" }, "setting", Some(format!("{friendly_dev}: tank capacity set to {} L.", fmt_num(l, 1.0, None)))));
        }
        ("POST", ["devices", _, "maretron", "action"]) => {
            let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let pc = c.before.and_then(|d| d.product.as_ref()).and_then(|p| p.product_code).unwrap_or(0);
            let info = crate::maretron_actions::find(id, pc);
            let val = b.get("value").and_then(|v| v.as_f64()).map(|v| fmt_num(v, 0.01, None));
            let kind = if info.map(|i| i.control).unwrap_or(true) { "setting" } else { "control" };
            out.push(base(info.map(|i| i.label.to_string()).unwrap_or_else(|| id.to_string()), None, val, "Maretron 126720 via 126208", "sent", kind, info.map(|i| format!("{friendly_dev}: {}.", i.label.to_lowercase()))));
        }
        ("PUT", ["devices", _, "config"]) => {
            let schema = b.get("schema").and_then(|v| v.as_str()).unwrap_or("configuration");
            let vals = b.get("values").map(|v| v.to_string()).unwrap_or_default();
            out.push(base(format!("Configuration · {schema}"), None, Some(vals.chars().take(120).collect()), "Maretron 126720 via 126208", "sent", "setting", Some(format!("{friendly_dev}: configuration updated."))));
        }
        ("POST", ["devices", _, "maretron"]) => {
            let cmd = b.get("command").and_then(|v| v.as_u64()).unwrap_or(0);
            out.push(base(format!("Maretron command {cmd}"), None, b.get("data").and_then(|v| v.as_str()).map(|s| s.to_string()), "Maretron 126720 via 126208 (expert)", "sent", "raw", None));
        }
        ("POST", ["switch"]) => {
            let inst = b.get("instance").and_then(|v| v.as_u64()).unwrap_or(0);
            let ind = b.get("indicator").and_then(|v| v.as_u64()).unwrap_or(0);
            let on = b.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut ch = base(format!("Switch bank {inst}, channel {ind}"), None, Some(if on { "on" } else { "off" }.into()), "127502 Switch Bank Control", "sent", "control", None);
            ch.device = format!("Bank {inst}");
            out.push(ch);
        }
        ("POST", ["bus", "tx"]) => {
            let pgn = b.get("pgn").and_then(|v| v.as_u64()).map(|p| p.to_string()).or_else(|| b.get("can_id").and_then(|v| v.as_str()).map(|s| format!("id {s}"))).unwrap_or_default();
            let mut ch = base(format!("Raw message {pgn}"), None, b.get("data").and_then(|v| v.as_str()).map(|s| s.to_string()), "Raw transmit (expert)", "sent", "raw", None);
            ch.device = "Bus".into();
            out.push(ch);
        }
        _ => {}
    }
    out
}

/// Routes whose successful (or failed) calls are recorded in the change log.
pub fn is_change_route(method: &str, path: &str) -> bool {
    let p = path.trim_start_matches("/api/v1/");
    let parts: Vec<&str> = p.split('/').collect();
    matches!(
        (method, parts.as_slice()),
        ("PATCH", ["devices", _, "standard"])
            | ("POST", ["devices", _, "fields" | "data-instance" | "pgn-rate" | "address" | "tank-capacity" | "maretron"])
            | ("POST", ["devices", _, "maretron", "action"])
            | ("PUT", ["devices", _, "config"])
            | ("POST", ["switch"])
            | ("POST", ["bus", "tx"])
    )
}

// ------------------------------------------------------------------ capture filter

/// Copy a pcapng capture leaving out other vessels' AIS messages.
/// Returns (frames kept, frames removed).
pub fn strip_other_ais(src: &Path, dst: &Path, own: Option<u32>) -> std::io::Result<(u64, u64)> {
    let bytes = std::fs::read(src)?;
    let frames = keelwire_core::pcap::parse_bytes(&bytes).map_err(std::io::Error::other)?;
    let f = std::io::BufWriter::new(std::fs::File::create(dst)?);
    let mut w = keelwire_core::pcap::PcapngWriter::new(f, &format!("Keelwire {}", crate::VERSION), "n2k")?;
    let mut drop: BTreeMap<(u8, u32, u8), bool> = BTreeMap::new();
    let (mut kept, mut removed) = (0u64, 0u64);
    for fr in frames {
        let id = fr.id();
        let mut keep = true;
        if AIS_PGNS.contains(&id.pgn) {
            let d = fr.payload();
            if !d.is_empty() {
                let seq = d[0] >> 5;
                if d[0] & 0x1F == 0 {
                    let mmsi = if d.len() >= 7 { Some(u32::from_le_bytes([d[3], d[4], d[5], d[6]])) } else { None };
                    let other = own.is_none() || mmsi != own;
                    drop.insert((id.source, id.pgn, seq), other);
                }
                keep = !drop.get(&(id.source, id.pgn, seq)).copied().unwrap_or(true);
            }
        }
        if keep {
            w.write(fr.timestamp_us, &fr)?;
            kept += 1;
        } else {
            removed += 1;
        }
    }
    w.flush()?;
    Ok((kept, removed))
}

/// Remove other vessels' AIS from a boat file (devices' last messages and
/// the per-PGN message list).
pub fn strip_boat_ais(b: &mut crate::boatfile::BoatFile, own: Option<u32>) {
    let mmsi_of = |data: &[u8]| if data.len() >= 5 { Some(u32::from_le_bytes([data[1], data[2], data[3], data[4]])) } else { None };
    b.messages.retain(|m| !AIS_PGNS.contains(&m.pgn) || (own.is_some() && mmsi_of(&m.data) == own));
    for d in &mut b.devices {
        for (p, o) in d.observed.iter_mut() {
            if AIS_PGNS.contains(p) {
                if own.is_none() || mmsi_of(&o.last_data) != own {
                    o.last_data.clear();
                }
                o.keys.clear();
                o.key_data.clear();
                o.key_seen.clear();
            }
        }
    }
}

pub fn sha256_file(p: &Path) -> Option<String> {
    use sha2::Digest;
    let b = std::fs::read(p).ok()?;
    let h = sha2::Sha256::digest(&b);
    Some(h.iter().map(|x| format!("{x:02x}")).collect())
}

pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(model: &str, serial: &str, sw: &str, inst: &[(u32, &str)]) -> DeviceSnap {
        DeviceSnap {
            key: model.into(),
            address: Some(17),
            model: Some(model.into()),
            serial: Some(serial.into()),
            mfr_code: Some(137),
            software: Some(sw.into()),
            instances: inst.iter().map(|(p, k)| (*p, vec![k.to_string()])).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diff_finds_added_gone_software_and_settings() {
        let prev = vec![snap("TLM100", "1", "1.0", &[(127505, "Instance=1")]), snap("GPS100", "2", "1.0", &[])];
        let now = vec![snap("TLM100", "1", "1.1", &[(127505, "Instance=0")]), snap("DST110", "3", "1.0", &[])];
        let d = diff_devices(&prev, &now);
        let kinds: Vec<&str> = d.iter().map(|x| x.kind.as_str()).collect();
        assert!(kinds.contains(&"added") && kinds.contains(&"gone") && kinds.contains(&"software") && kinds.contains(&"setting"), "{d:?}");
    }

    #[test]
    fn vessel_ids_and_folders() {
        let v = Vessel { mmsi: Some(367550123), name: Some("SECOND WIND".into()), ..Default::default() };
        assert_eq!(v.id().unwrap(), "mmsi-367550123");
        assert_eq!(v.folder_name(), "Second Wind (367550123)");
        let h = Vessel { hin: Some("ser12345f526".into()), name: Some("Miss Ellie".into()), ..Default::default() };
        assert_eq!(h.id().unwrap(), "hin-SER12345F526");
        assert_eq!(h.folder_name(), "Miss Ellie (SER12345F526)");
    }

    #[test]
    fn change_routes() {
        assert!(is_change_route("PATCH", "/api/v1/devices/abc/standard"));
        assert!(is_change_route("POST", "/api/v1/devices/abc/fields"));
        assert!(!is_change_route("GET", "/api/v1/devices/abc"));
        assert!(!is_change_route("POST", "/api/v1/devices/abc/request"));
    }

    #[test]
    fn strips_other_ships_from_capture() {
        use keelwire_core::fastpacket::fragment;
        use keelwire_core::{CanId, RawCanFrame};
        let dir = std::env::temp_dir().join(format!("kw-strip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.pcapng");
        let dst = dir.join("b.pcapng");
        {
            let f = std::fs::File::create(&src).unwrap();
            let mut w = keelwire_core::pcap::PcapngWriter::new(f, "t", "n2k").unwrap();
            let id = CanId { priority: 4, pgn: 129039, destination: 255, source: 30 }.encode();
            for (seq, mmsi) in [(1u8, 367550123u32), (2, 338114902)] {
                let mut d = vec![18u8];
                d.extend_from_slice(&mmsi.to_le_bytes());
                d.extend_from_slice(&[0u8; 22]);
                for fr in fragment(seq, &d) {
                    w.write(1_000_000, &RawCanFrame::new(1_000_000, id, &fr)).unwrap();
                }
            }
            let other = CanId { priority: 2, pgn: 129025, destination: 255, source: 10 }.encode();
            w.write(1_000_000, &RawCanFrame::new(1_000_000, other, &[0; 8])).unwrap();
            w.flush().unwrap();
        }
        let (kept, removed) = strip_other_ais(&src, &dst, Some(367550123)).unwrap();
        assert_eq!(removed, 4, "the other ship's 4 frames are removed");
        assert_eq!(kept, 5, "own 4 frames + the GPS frame are kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lat_lon() {
        assert_eq!(fmt_lat_lon(29.556833, -95.020333), "29°33.41′N 095°01.22′W");
    }
}
