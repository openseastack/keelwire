//! Built-in simulator: a small synthetic vessel network.
//!
//! Devices claim addresses, broadcast realistic periodic data, answer ISO
//! Requests (60928/126996/126998/126464), NMEA Group Function requests and
//! commands (126208), Switch Bank Control (127502) and Commanded Address
//! (65240), and defend their addresses. `sim:?conflict=1` makes both tank
//! senders use instance 0 to exercise the instance-conflict analyzer.

use std::collections::HashMap;
use std::time::Duration;

use keelwire_core::bits::set_bits;
use keelwire_core::fastpacket::{fragment, Reassembler, SequenceCounter};
use keelwire_core::groupfunction::GroupFunction;
use keelwire_core::iso::{self, ConfigInfo, IsoName, ProductInfo};
use keelwire_core::isotp::TpTracker;
use keelwire_core::proprietary::{maretron, ProprietaryHeader};
use keelwire_core::{CanId, N2kMessage, RawCanFrame};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{clock, BusEvent, CanInterface, InterfaceCaps, InterfaceStatus, Outbound, Uri};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Gps,
    Compass,
    Wind,
    Depth,
    Engine,
    Battery,
    Tank(u8),
    Clmd16,
    Ckm12,
    /// Class B AIS transponder: this boat's own reports plus other ships it hears.
    Ais,
}

struct SimDevice {
    kind: Kind,
    address: u8,
    name: IsoName,
    product: ProductInfo,
    config: ConfigInfo,
    tx: Vec<u32>,
    rx: Vec<u32>,
    hb_seq: u8,
    /// data instance used in instanced PGNs
    instance: u8,
    /// tank capacity in litres (TLM)
    capacity_l: f64,
    /// tank fluid type (127505 field 2)
    fluid: u8,
    /// depth transducer offset in mm (128267 field 3)
    depth_offset_mm: i16,
    /// temperature source (130316 field 3, 0 = sea)
    temp_source: u8,
    /// Maretron compass settings (SSC300)
    var_src: u8,
    rot_ms: u16,
    orientation: u16,
    direction: u16,
}

pub struct Simulator {
    conflict: bool,
    rate: f64,
}

impl Simulator {
    pub fn new(u: &Uri) -> Self {
        Simulator { conflict: u.flag("conflict"), rate: u.q("rate").unwrap_or(1.0f64).clamp(0.1, 10.0) }
    }
}

fn product(model: &str, code: u16, sw: &str, serial: &str, len: u8) -> ProductInfo {
    ProductInfo {
        nmea2000_version: Some(2100),
        product_code: Some(code),
        model_id: model.into(),
        software_version: sw.into(),
        model_version: "Rev 1".into(),
        serial_code: serial.into(),
        certification_level: Some(2),
        load_equivalency: Some(len),
    }
}

fn name(unique: u32, mfr: u16, class: u8, function: u8, instance: u8) -> IsoName {
    let mut n = IsoName { unique_number: unique, manufacturer_code: mfr, device_class: class, device_function: function, industry_group: 4, arbitrary_address_capable: true, ..Default::default() };
    n.set_device_instance(instance);
    n
}

fn devices(conflict: bool) -> Vec<SimDevice> {
    let base_rx = vec![59392, 59904, 60928, 65240, 126208, 126464, 126996, 126998];
    let mk = |kind, address, n: IsoName, p: ProductInfo, tx: Vec<u32>, instance| SimDevice {
        kind,
        address,
        name: n,
        product: p,
        config: ConfigInfo { installation_description1: String::new(), installation_description2: String::new(), manufacturer_information: "Keelwire simulator".into() },
        tx: [vec![59392, 60928, 126208, 126464, 126993, 126996, 126998], tx].concat(),
        rx: base_rx.clone(),
        hb_seq: 0,
        instance,
        capacity_l: 400.0,
        fluid: 0,
        depth_offset_mm: 500,
        temp_source: 0,
        var_src: 1,
        rot_ms: 1000,
        orientation: 1,
        direction: 1,
    };
    vec![
        mk(Kind::Gps, 10, name(100_001, 137, 60, 145, 0), product("GPS200", 3373, "1.4.2", "1290001", 2), vec![129025, 129026, 129029, 126992], 0),
        mk(Kind::Compass, 11, name(100_002, 137, 60, 140, 0), product("SSC300", 2686, "2.1.0", "1290002", 2), vec![127250, 127251, 127257], 0),
        mk(Kind::Wind, 12, name(100_003, 137, 85, 130, 0), product("WSO100", 0, "1.0.8", "1290003", 1), vec![130306, 130311], 0),
        mk(Kind::Depth, 13, name(100_004, 137, 60, 130, 0), product("DST110", 1534, "1.3.0", "1290004", 1), vec![128267, 128259, 130316], 0),
        mk(Kind::Engine, 14, name(100_005, 137, 50, 140, 0), product("J2K100", 4319, "1.6.3", "1290005", 1), vec![127488, 127489], 0),
        mk(Kind::Battery, 15, name(100_006, 137, 35, 170, 0), product("DCM100", 9375, "1.1.4", "1290006", 1), vec![127506, 127508], 0),
        mk(Kind::Tank(0), 16, name(100_007, 137, 75, 150, 0), product("TLM100", 27725, "1.2.0", "1290007", 1), vec![127505], 0),
        mk(Kind::Tank(1), 17, name(100_008, 137, 75, 150, 1), product("TLM100", 27725, "1.2.0", "1290008", 1), vec![127505], if conflict { 0 } else { 1 }),
        mk(Kind::Clmd16, 20, name(100_009, 137, 30, 140, 0), product("CLMD16", 12337, "1.0.5", "1290009", 1), vec![127501, 65284], 0),
        mk(Kind::Ckm12, 21, name(100_010, 137, 110, 130, 1), product("CKM12", 0, "1.0.2", "1290010", 1), vec![127501], 1),
        mk(Kind::Ais, 30, name(100_011, 504, 60, 195, 0), product("XB-8000", 8000, "4.2.1", "1290011", 2), vec![129038, 129039, 129794, 129809, 129810], 0),
    ]
}

/// The simulated boat's own MMSI (from its AIS transponder).
pub const OWN_MMSI: u32 = 367_550_123;

const OTHER_SHIPS: [(u32, &str, &str, f64, f64); 3] =
    [(338_114_902, "BLUE HERON", "WDF3311", 26.0150, -80.1201), (367_221_480, "SEA DUCHESS", "WDC9081", 26.0091, -80.1302), (316_004_512, "NORTHERN STAR", "VCX2210", 26.0203, -80.1180)];

fn ais_text(d: &mut Vec<u8>, at: usize, chars: usize, text: &str) {
    let b: Vec<u8> = text.bytes().chain(std::iter::repeat(b'@')).take(chars).collect();
    for (i, c) in b.iter().enumerate() {
        set_bits(d, at + i * 8, 8, *c as u64);
    }
}

/// 129039 AIS Class B Position Report.
fn ais_class_b_position(mmsi: u32, lat: f64, lon: f64, cog: f64, sog_kn: f64, transceiver: u64) -> Vec<u8> {
    let mut d = vec![0u8; 27];
    set_bits(&mut d, 0, 6, 18);
    set_bits(&mut d, 8, 32, mmsi as u64);
    set_bits(&mut d, 40, 32, ((lon * 1e7) as i32) as u32 as u64);
    set_bits(&mut d, 72, 32, ((lat * 1e7) as i32) as u32 as u64);
    set_bits(&mut d, 104, 1, 1);
    set_bits(&mut d, 106, 6, 30);
    set_bits(&mut d, 112, 16, (cog * 10_000.0) as u64);
    set_bits(&mut d, 128, 16, (sog_kn * 0.514_444 * 100.0) as u64);
    set_bits(&mut d, 144, 19, 0x60000);
    set_bits(&mut d, 163, 5, transceiver);
    set_bits(&mut d, 168, 16, 0xFFFF);
    set_bits(&mut d, 184, 8, 0);
    set_bits(&mut d, 194, 7, 0b0011100);
    set_bits(&mut d, 201, 7, 0);
    set_bits(&mut d, 208, 8, 0xFF);
    d
}

/// 129809 AIS Class B static data, part A (name).
fn ais_static_a(mmsi: u32, name: &str, transceiver: u64) -> Vec<u8> {
    let mut d = vec![0u8; 27];
    set_bits(&mut d, 0, 6, 24);
    set_bits(&mut d, 8, 32, mmsi as u64);
    ais_text(&mut d, 40, 20, name);
    set_bits(&mut d, 200, 5, transceiver);
    set_bits(&mut d, 208, 8, 0xFF);
    d
}

/// 129810 AIS Class B static data, part B (type, call sign, dimensions).
fn ais_static_b(mmsi: u32, ship_type: u64, call: &str, length_m: f64, beam_m: f64, transceiver: u64) -> Vec<u8> {
    let mut d = vec![0u8; 35];
    set_bits(&mut d, 0, 6, 24);
    set_bits(&mut d, 8, 32, mmsi as u64);
    set_bits(&mut d, 40, 8, ship_type);
    ais_text(&mut d, 48, 7, "VSPRMRN");
    ais_text(&mut d, 104, 7, call);
    set_bits(&mut d, 160, 16, (length_m * 10.0) as u64);
    set_bits(&mut d, 176, 16, (beam_m * 10.0) as u64);
    set_bits(&mut d, 192, 16, 20);
    set_bits(&mut d, 208, 16, 60);
    set_bits(&mut d, 224, 32, 0);
    set_bits(&mut d, 260, 4, 1);
    set_bits(&mut d, 264, 5, transceiver);
    set_bits(&mut d, 272, 8, 0xFF);
    d
}

struct State {
    devs: Vec<SimDevice>,
    seq: SequenceCounter,
    banks: HashMap<u8, [u8; 28]>,
    tick: u64,
    rng: u64,
    rx_fp: Reassembler,
    rx_tp: TpTracker,
}

fn prio_for(pgn: u32) -> u8 {
    match pgn {
        127250 | 127251 | 129025 | 129026 | 127488 | 130306 => 2,
        127245 | 127257 | 127501 | 127502 => 3,
        126208 | 126464 | 126996 | 126998 | 59904 | 60928 | 59392 => 6,
        _ => 6,
    }
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn i16le(v: &mut Vec<u8>, x: i16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn i32le(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

impl State {
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % 10_000) as f64 / 10_000.0
    }

    fn frames_for(&mut self, src: u8, dst: u8, pgn: u32, data: &[u8], fast: bool) -> Vec<RawCanFrame> {
        let id = CanId { priority: prio_for(pgn), pgn, destination: dst, source: src }.encode();
        let now = clock::now_us();
        if fast {
            let seq = self.seq.next(pgn ^ (src as u32) << 24);
            fragment(seq, data).iter().map(|f| RawCanFrame::new(now, id, f)).collect()
        } else {
            vec![RawCanFrame::new(now, id, data)]
        }
    }

    fn claim(&mut self, i: usize) -> Vec<RawCanFrame> {
        let d = &self.devs[i];
        let (a, n) = (d.address, d.name.to_bytes());
        self.frames_for(a, 255, iso::PGN_ADDRESS_CLAIM, &n, false)
    }

    fn periodic(&mut self, i: usize) -> Vec<RawCanFrame> {
        let t = self.tick as f64 / 10.0; // seconds
        let tick = self.tick;
        let kind = self.devs[i].kind;
        let addr = self.devs[i].address;
        let inst = self.devs[i].instance;
        let mut out = Vec::new();
        let mut send = |s: &mut State, pgn: u32, data: Vec<u8>, fast: bool| {
            out.extend(s.frames_for(addr, 255, pgn, &data, fast));
        };
        let sid = (tick % 250) as u8;
        match kind {
            Kind::Gps => {
                let lat = 26.0123 + 0.0001 * (t / 60.0).sin();
                let lon = -80.1234 + 0.0001 * (t / 60.0).cos();
                let mut d = Vec::new();
                i32le(&mut d, (lat * 1e7) as i32);
                i32le(&mut d, (lon * 1e7) as i32);
                send(self, 129025, d, false);
                if tick.is_multiple_of(3) {
                    let mut d = vec![sid, 0xFC];
                    u16le(&mut d, ((1.2 + 0.05 * (t / 7.0).sin()) * 10000.0) as u16);
                    u16le(&mut d, ((3.1 + 0.2 * self.rand()) * 100.0) as u16);
                    d.extend_from_slice(&[0xFF, 0xFF]);
                    send(self, 129026, d, false);
                }
                if tick.is_multiple_of(10) {
                    let mut d = vec![sid];
                    u16le(&mut d, 20_384); // days since 1970
                    d.extend_from_slice(&((((t as u64) % 86_400) * 10_000) as u32).to_le_bytes());
                    d.extend_from_slice(&((lat * 1e16) as i64).to_le_bytes());
                    d.extend_from_slice(&((lon * 1e16) as i64).to_le_bytes());
                    d.extend_from_slice(&(150_000i64).to_le_bytes()); // altitude 1e-6 m
                    d.push(0x13); // GPS, GNSS fix
                    d.push(0xFC);
                    d.push(9); // satellites
                    i16le(&mut d, 90); // HDOP 0.9
                    i16le(&mut d, 150); // PDOP
                    i32le(&mut d, -2_500); // geoidal separation
                    d.push(0);
                    send(self, 129029, d, true);
                    let mut d = vec![sid, 0xF0];
                    u16le(&mut d, 20_384);
                    d.extend_from_slice(&((((t as u64) % 86_400) * 10_000) as u32).to_le_bytes());
                    send(self, 126992, d, false);
                }
            }
            Kind::Compass => {
                let hdg = ((t * 0.02).sin() * 0.3 + 1.2).rem_euclid(std::f64::consts::TAU);
                let mut d = vec![sid];
                u16le(&mut d, (hdg * 10000.0) as u16);
                i16le(&mut d, 0x7FFF);
                i16le(&mut d, -1_200);
                d.push(0xFD);
                send(self, 127250, d, false);
                if tick.is_multiple_of(2) {
                    let mut d = vec![sid];
                    i32le(&mut d, ((t * 0.02).cos() * 0.006 / 3.125e-8) as i32);
                    d.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
                    send(self, 127251, d, false);
                }
                if tick.is_multiple_of(10) {
                    let mut d = vec![sid];
                    i16le(&mut d, 0x7FFF);
                    i16le(&mut d, ((t * 0.3).sin() * 0.05 * 10000.0) as i16);
                    i16le(&mut d, ((t * 0.5).sin() * 0.08 * 10000.0) as i16);
                    d.push(0xFF);
                    send(self, 127257, d, false);
                }
            }
            Kind::Wind => {
                if tick.is_multiple_of(2) {
                    let mut d = vec![sid];
                    u16le(&mut d, ((7.0 + 2.0 * self.rand()) * 100.0) as u16);
                    u16le(&mut d, ((0.8 + 0.1 * (t / 5.0).sin()) * 10000.0) as u16);
                    d.extend_from_slice(&[0xFA, 0xFF, 0xFF]); // apparent
                    send(self, 130306, d, false);
                }
                if tick.is_multiple_of(20) {
                    let mut d = vec![sid, 0x41];
                    u16le(&mut d, ((26.0 + 273.15) * 100.0) as u16);
                    u16le(&mut d, (72.0 / 0.004) as u16);
                    u16le(&mut d, 1013);
                    send(self, 130311, d, false);
                }
            }
            Kind::Depth => {
                if tick.is_multiple_of(10) {
                    let mut d = vec![sid];
                    d.extend_from_slice(&(((12.5 + (t / 30.0).sin()) * 100.0) as u32).to_le_bytes());
                    i16le(&mut d, self.devs[i].depth_offset_mm);
                    d.push(0xFF);
                    send(self, 128267, d, false);
                    let mut d = vec![sid];
                    u16le(&mut d, ((3.0 + 0.1 * self.rand()) * 100.0) as u16);
                    u16le(&mut d, 0xFFFF);
                    d.extend_from_slice(&[0x00, 0xFF, 0xFF, 0xFF]);
                    send(self, 128259, d, false);
                }
                if tick % 20 == 5 {
                    let mut d = vec![sid, inst, self.devs[i].temp_source];
                    let k = ((27.5 + 273.15) * 1000.0) as u32;
                    d.extend_from_slice(&k.to_le_bytes()[..3]);
                    d.extend_from_slice(&[0xFF, 0xFF]);
                    send(self, 130316, d, false);
                }
            }
            Kind::Engine => {
                let rpm = 1800.0 + 50.0 * (t / 4.0).sin();
                let mut d = vec![inst];
                u16le(&mut d, (rpm / 0.25) as u16);
                u16le(&mut d, 0xFFFF);
                d.extend_from_slice(&[0x7F, 0xFF, 0xFF]);
                send(self, 127488, d, false);
                if tick.is_multiple_of(5) {
                    let mut d = vec![inst];
                    u16le(&mut d, 4000); // oil pressure 400 kPa (hPa units)
                    u16le(&mut d, ((90.0 + 273.15) * 10.0) as u16);
                    u16le(&mut d, ((82.0 + 273.15) * 100.0) as u16);
                    i16le(&mut d, 1410); // alternator 14.1 V
                    i16le(&mut d, 1200); // fuel rate 0.1 L/h
                    d.extend_from_slice(&((1234u32 * 3600 + (t as u32)).to_le_bytes()));
                    u16le(&mut d, 0xFFFF);
                    u16le(&mut d, 0xFFFF);
                    d.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x7F, 0x7F]);
                    send(self, 127489, d, true);
                }
            }
            Kind::Battery => {
                if tick.is_multiple_of(10) {
                    let mut d = vec![inst];
                    u16le(&mut d, (1285.0 + 10.0 * (t / 60.0).sin()) as u16);
                    i16le(&mut d, (-52.0 + 5.0 * self.rand()) as i16);
                    u16le(&mut d, ((24.0 + 273.15) * 100.0) as u16);
                    d.push(sid);
                    send(self, 127508, d, false);
                }
                if tick % 15 == 3 {
                    let mut d = vec![sid, inst, 0, 87, 100];
                    u16le(&mut d, 600);
                    u16le(&mut d, 0xFFFF);
                    send(self, 127506, d, true);
                }
            }
            Kind::Tank(_) => {
                if tick % 25 == (addr as u64 % 25) {
                    let mut d = vec![0u8];
                    // instance (4 bits) + fluid type (4 bits, 0 = fuel)
                    set_bits(&mut d, 0, 4, inst as u64);
                    set_bits(&mut d, 4, 4, self.devs[i].fluid as u64);
                    let lvl = 62.0 - (t / 600.0) + 0.3 * self.rand() + inst as f64 * 10.0;
                    i16le(&mut d, (lvl / 0.004) as i16);
                    let cap = self.devs[i].capacity_l;
                    d.extend_from_slice(&((cap * 10.0) as u32).to_le_bytes());
                    d.push(0xFF);
                    send(self, 127505, d, false);
                }
            }
            Kind::Clmd16 => {
                if tick.is_multiple_of(20) {
                    let st = *self.banks.get(&0).unwrap_or(&[3; 28]);
                    send(self, 127501, bank_payload(0, &st), false);
                }
                // one breaker current per tick, cycling through 16 channels
                let ch = (tick % 16) as u8;
                let on = self.banks.get(&0).map(|b| b[ch as usize] == 1).unwrap_or(false);
                let amps = if on { 1.5 + ch as f64 * 0.4 + 0.2 * self.rand() } else { 0.0 };
                let mut d = ProprietaryHeader::MARETRON.encode().to_vec();
                d.extend_from_slice(&[0, ch + 1]);
                u16le(&mut d, (amps * 10.0) as u16);
                d.extend_from_slice(&[0xFF, 0xFF]);
                send(self, 65284, d, false);
            }
            Kind::Ckm12 => {
                if tick % 20 == 10 {
                    let st = *self.banks.get(&inst).unwrap_or(&[3; 28]);
                    send(self, 127501, bank_payload(inst, &st), false);
                }
            }
            Kind::Ais => {
                // Own boat: position every 10 s, static data every 30 s,
                // transceiver information = channel A/B VDL transmission.
                if tick % 100 == 40 {
                    let own = 2 + (tick / 100) % 2;
                    send(self, 129039, ais_class_b_position(OWN_MMSI, 26.0123, -80.1234, 0.0, 0.0, own), true);
                }
                if tick % 300 == 70 {
                    send(self, 129809, ais_static_a(OWN_MMSI, "SECOND WIND", 2), true);
                    send(self, 129810, ais_static_b(OWN_MMSI, 36, "WDK4721", 12.8, 4.1, 3), true);
                }
                // Other ships heard on VHF (transceiver = reception).
                for (k, (mmsi, nm, call, lat, lon)) in OTHER_SHIPS.iter().enumerate() {
                    let phase = 15 + k as u64 * 23;
                    if tick % 150 == phase % 150 {
                        let drift = 0.002 * (t / 300.0 + k as f64).sin();
                        send(self, 129039, ais_class_b_position(*mmsi, lat + drift, lon - drift, 1.2, 4.5, (k % 2) as u64), true);
                    }
                    if tick % 600 == (phase * 3) % 600 {
                        send(self, 129809, ais_static_a(*mmsi, nm, (k % 2) as u64), true);
                        send(self, 129810, ais_static_b(*mmsi, 37, call, 10.5 + k as f64, 3.6, (k % 2) as u64), true);
                    }
                }
            }
        }
        if tick % 600 == (i as u64 * 7) % 600 {
            self.devs[i].hb_seq = self.devs[i].hb_seq.wrapping_add(1);
            let hb = iso::Heartbeat { interval_ms: Some(60_000), sequence: self.devs[i].hb_seq }.encode();
            out.extend(self.frames_for(addr, 255, iso::PGN_HEARTBEAT, &hb, false));
        }
        out
    }

    fn respond_iso_request(&mut self, i: usize, requester: u8, pgn: u32) -> Vec<RawCanFrame> {
        let addr = self.devs[i].address;
        let dst = requester;
        match pgn {
            iso::PGN_ADDRESS_CLAIM => self.claim(i),
            iso::PGN_PRODUCT_INFO => {
                let p = self.devs[i].product.encode();
                self.frames_for(addr, dst, pgn, &p, true)
            }
            iso::PGN_CONFIG_INFO => {
                let c = self.devs[i].config.encode();
                self.frames_for(addr, dst, pgn, &c, true)
            }
            iso::PGN_PGN_LIST => {
                let (tx, rx) = (self.devs[i].tx.clone(), self.devs[i].rx.clone());
                let mut f = self.frames_for(addr, dst, pgn, &iso::encode_pgn_list(0, &tx), true);
                f.extend(self.frames_for(addr, dst, pgn, &iso::encode_pgn_list(1, &rx), true));
                f
            }
            p if self.devs[i].tx.contains(&p) => vec![],
            _ => {
                // NAK
                let ack = iso::iso_ack(1, pgn);
                self.frames_for(addr, dst, iso::PGN_ISO_ACK, &ack, false)
            }
        }
    }

    fn group_function(&mut self, i: usize, requester: u8, data: &[u8]) -> Vec<RawCanFrame> {
        let width = |pgn: u32, field: u8, rest: &[u8]| -> Option<usize> {
            match (pgn, field) {
                (iso::PGN_CONFIG_INFO, 1..=3) => rest.first().map(|&l| (l as usize).max(2)),
                (iso::PGN_PGN_LIST, 1) => Some(1),
                (iso::PGN_ADDRESS_CLAIM, 1) => Some(3),
                (iso::PGN_ADDRESS_CLAIM, 2) => Some(2),
                (iso::PGN_ADDRESS_CLAIM, _) => Some(1),
                (127501, _) => Some(1),
                (127505, 4) => Some(4),
                (127505, _) => Some(1),
                (128267, 2) => Some(4),
                (128267, 3) => Some(2),
                (130316, 4) => Some(3),
                (130316, 5) => Some(2),
                _ => Some(1),
            }
        };
        if let Some((product, _sw, cmd, mdata)) = maretron::parse_command_payload(data) {
            return self.maretron_command(i, requester, product, cmd, &mdata);
        }
        let Some(g) = GroupFunction::decode(data, &width) else { return vec![] };
        let addr = self.devs[i].address;
        let mut out = Vec::new();
        let ack = |pgn: u32, pgn_error: u8, n: usize, perr: u8| GroupFunction::Ack { pgn, pgn_error, interval_error: 0, param_errors: vec![perr; n] }.encode();
        match g {
            GroupFunction::Request { pgn, params, .. } => {
                if pgn == iso::PGN_PGN_LIST {
                    let which = params.iter().find(|(f, _)| *f == 1).and_then(|(_, v)| v.first().copied());
                    let (tx, rx) = (self.devs[i].tx.clone(), self.devs[i].rx.clone());
                    if which != Some(1) {
                        out.extend(self.frames_for(addr, requester, pgn, &iso::encode_pgn_list(0, &tx), true));
                    }
                    if which != Some(0) {
                        out.extend(self.frames_for(addr, requester, pgn, &iso::encode_pgn_list(1, &rx), true));
                    }
                } else if self.devs[i].tx.contains(&pgn) {
                    out.extend(self.frames_for(addr, requester, 126208, &ack(pgn, 0, params.len(), 0), true));
                } else {
                    out.extend(self.frames_for(addr, requester, 126208, &ack(pgn, 1, params.len(), 5), true));
                }
            }
            GroupFunction::Command { pgn, params, .. } => {
                let mut errs = Vec::new();
                for (f, v) in &params {
                    errs.push(self.apply_command(i, pgn, *f, v));
                }
                let pgn_error = if errs.iter().all(|e| *e == 0) || matches!(pgn, iso::PGN_ADDRESS_CLAIM | iso::PGN_CONFIG_INFO | 127501) { 0 } else { 1 };
                let a = GroupFunction::Ack { pgn, pgn_error, interval_error: 0, param_errors: errs }.encode();
                out.extend(self.frames_for(addr, requester, 126208, &a, true));
                if pgn == iso::PGN_ADDRESS_CLAIM {
                    out.extend(self.claim(i));
                }
                if pgn == 127501 {
                    let inst = self.devs[i].instance;
                    let st = *self.banks.get(&inst).unwrap_or(&[3; 28]);
                    out.extend(self.frames_for(addr, 255, 127501, &bank_payload(inst, &st), false));
                }
            }
            GroupFunction::Read { pgn, .. } | GroupFunction::Write { pgn, .. } => {
                out.extend(self.frames_for(addr, requester, 126208, &ack(pgn, 6, 0, 0), true));
            }
            _ => {}
        }
        out
    }

    /// Maretron command wrapped in 126208 (see keelwire_core::proprietary::maretron).
    fn maretron_command(&mut self, i: usize, requester: u8, product: u16, cmd: u8, data: &[u8]) -> Vec<RawCanFrame> {
        use maretron::command as c;
        let addr = self.devs[i].address;
        let own = self.devs[i].product.product_code.unwrap_or(0);
        let ack = |err: u8| GroupFunction::Ack { pgn: maretron::PGN_CONFIG, pgn_error: err, interval_error: 0, param_errors: vec![0, 0, 0, 0] }.encode();
        let mut out = Vec::new();
        if product != own || self.devs[i].name.manufacturer_code != maretron::MANUFACTURER_CODE {
            out.extend(self.frames_for(addr, requester, 126208, &ack(1), true));
            return out;
        }
        let kind = self.devs[i].kind;
        let reply: Option<Vec<u8>> = match (kind, cmd) {
            (Kind::Compass, c::COMPASS_VARIATION_SOURCE) => {
                if let Some(&src) = data.first() {
                    if src != 0xFF && src <= 3 {
                        self.devs[i].var_src = src;
                    }
                }
                let mut p = vec![self.devs[i].var_src];
                p.extend_from_slice(&(-1200i16).to_le_bytes());
                Some(p)
            }
            (Kind::Compass, c::COMPASS_ROT_CONFIG) if data.len() >= 2 => {
                let sub = u16::from_le_bytes([data[0], data[1]]);
                let d = &mut self.devs[i];
                match sub {
                    maretron::rot::GET_DAMPING => Some([&sub.to_le_bytes()[..], &d.rot_ms.to_le_bytes()].concat()),
                    maretron::rot::SET_DAMPING if data.len() >= 4 => {
                        d.rot_ms = u16::from_le_bytes([data[2], data[3]]);
                        Some([&sub.to_le_bytes()[..], &d.rot_ms.to_le_bytes()].concat())
                    }
                    maretron::rot::SET_INSTALLATION if data.len() >= 6 => {
                        d.orientation = u16::from_le_bytes([data[2], data[3]]);
                        d.direction = u16::from_le_bytes([data[4], data[5]]);
                        Some([&maretron::rot::GET_INSTALLATION.to_le_bytes()[..], &d.orientation.to_le_bytes(), &d.direction.to_le_bytes()].concat())
                    }
                    maretron::rot::GET_INSTALLATION => Some([&sub.to_le_bytes()[..], &d.orientation.to_le_bytes(), &d.direction.to_le_bytes()].concat()),
                    _ => None,
                }
            }
            (Kind::Compass, c::COMPASS_RESEND_CALIBRATION_STATUS) => Some(vec![2]),
            // Calibration start is reported as a calibration status ("started").
            (Kind::Compass, c::COMPASS_RESET_START_CALIBRATION) => Some(vec![1]),
            (Kind::Compass, c::COMPASS_INSTALLATION_OFFSET) => None,
            (Kind::Tank(_), c::TANK_GET_LEVEL_CONFIG) => {
                let row = data.first().copied().unwrap_or(0).min(15);
                let mut p = vec![0u8, row];
                p.extend_from_slice(&(row as u32 * 100).to_le_bytes());
                p.extend_from_slice(&(((row as f64 / 15.0 * 100.0) / 0.004) as u16).to_le_bytes());
                Some(p)
            }
            _ => {
                out.extend(self.frames_for(addr, requester, 126208, &ack(4), true));
                return out;
            }
        };
        out.extend(self.frames_for(addr, requester, 126208, &ack(0), true));
        if let Some(payload) = reply {
            let resp_cmd = if cmd == c::COMPASS_RESET_START_CALIBRATION { c::COMPASS_RESEND_CALIBRATION_STATUS } else { cmd };
            let m = maretron::ConfigMessage { product_code: own, software_code: 1, opcode: resp_cmd, payload };
            out.extend(self.frames_for(addr, requester, maretron::PGN_CONFIG, &m.encode(), true));
        }
        out
    }

    /// Returns parameter error code (0 = ok).
    fn apply_command(&mut self, i: usize, pgn: u32, field: u8, v: &[u8]) -> u8 {
        let d = &mut self.devs[i];
        match (pgn, field) {
            (iso::PGN_ADDRESS_CLAIM, 3) => {
                d.name.device_instance_lower = v.first().copied().unwrap_or(0) & 7;
                0
            }
            (iso::PGN_ADDRESS_CLAIM, 4) => {
                d.name.device_instance_upper = v.first().copied().unwrap_or(0) & 0x1F;
                0
            }
            (iso::PGN_ADDRESS_CLAIM, 8) => {
                d.name.system_instance = v.first().copied().unwrap_or(0) & 0xF;
                0
            }
            (iso::PGN_CONFIG_INFO, 1) | (iso::PGN_CONFIG_INFO, 2) => {
                let s = iso::string_lau(v).map(|x| x.0).unwrap_or_default();
                if field == 1 {
                    d.config.installation_description1 = s;
                } else {
                    d.config.installation_description2 = s;
                }
                0
            }
            (127501, 1) if matches!(d.kind, Kind::Clmd16) => {
                if v.first() == Some(&d.instance) {
                    0
                } else {
                    3
                }
            }
            (127505, 4) if matches!(d.kind, Kind::Tank(_)) && v.len() >= 4 => {
                d.capacity_l = u32::from_le_bytes([v[0], v[1], v[2], v[3]]) as f64 / 10.0;
                0
            }
            (127501, f) if (2..=29).contains(&f) && matches!(d.kind, Kind::Clmd16) => {
                let inst = d.instance;
                let b = self.banks.entry(inst).or_insert([3; 28]);
                b[(f - 2) as usize] = v.first().copied().unwrap_or(0) & 1;
                0
            }
            (127505, 1) if matches!(d.kind, Kind::Tank(_)) => {
                d.instance = v.first().copied().unwrap_or(0) & 0x0F;
                0
            }
            (127505, 2) if matches!(d.kind, Kind::Tank(_)) => {
                d.fluid = v.first().copied().unwrap_or(0) & 0x0F;
                0
            }
            (128267, 3) if matches!(d.kind, Kind::Depth) && v.len() >= 2 => {
                d.depth_offset_mm = i16::from_le_bytes([v[0], v[1]]);
                0
            }
            (130316, 2) if matches!(d.kind, Kind::Depth) => {
                d.instance = v.first().copied().unwrap_or(0);
                0
            }
            (130316, 3) if matches!(d.kind, Kind::Depth) => {
                d.temp_source = v.first().copied().unwrap_or(0);
                0
            }
            _ => 5,
        }
    }

    fn handle_message(&mut self, m: &N2kMessage) -> Vec<RawCanFrame> {
        let mut out = Vec::new();
        let targets: Vec<usize> = (0..self.devs.len()).filter(|&i| m.destination == 255 || m.destination == self.devs[i].address).collect();
        match m.pgn {
            iso::PGN_ISO_REQUEST => {
                if let Some(p) = iso::parse_iso_request(&m.data) {
                    for i in targets {
                        out.extend(self.respond_iso_request(i, m.source, p));
                    }
                }
            }
            iso::PGN_GROUP_FUNCTION => {
                for i in targets {
                    if m.destination != 255 {
                        out.extend(self.group_function(i, m.source, &m.data));
                    }
                }
            }
            127502 => {
                if let Some((inst, st)) = iso::parse_switch_bank(&m.data) {
                    if let Some(i) = self.devs.iter().position(|d| d.kind == Kind::Clmd16 && d.instance == inst) {
                        let b = self.banks.entry(inst).or_insert([3; 28]);
                        for (k, s) in st.iter().enumerate().take(16) {
                            if *s <= 1 {
                                b[k] = *s;
                            }
                        }
                        let st = *b;
                        let addr = self.devs[i].address;
                        out.extend(self.frames_for(addr, 255, 127501, &bank_payload(inst, &st), false));
                    }
                }
            }
            iso::PGN_ADDRESS_CLAIM => {
                // Defend or yield.
                if let Some(other) = IsoName::from_bytes(&m.data) {
                    if let Some(i) = self.devs.iter().position(|d| d.address == m.source) {
                        if self.devs[i].name.to_u64() < other.to_u64() {
                            out.extend(self.claim(i));
                        } else {
                            let used: Vec<u8> = self.devs.iter().map(|d| d.address).collect();
                            let new = (30..=250u8).find(|a| !used.contains(a) && *a != m.source).unwrap_or(254);
                            self.devs[i].address = new;
                            out.extend(self.claim(i));
                        }
                    }
                }
            }
            iso::PGN_COMMANDED_ADDRESS
                if m.data.len() >= 9 => {
                    let n = u64::from_le_bytes(m.data[..8].try_into().unwrap());
                    if let Some(i) = self.devs.iter().position(|d| d.name.to_u64() == n) {
                        self.devs[i].address = m.data[8];
                        out.extend(self.claim(i));
                    }
                }
            _ => {}
        }
        out
    }

    fn handle_frame(&mut self, f: &RawCanFrame) -> Vec<RawCanFrame> {
        let id = f.id();
        let now = clock::now_us();
        if id.pgn == keelwire_core::isotp::PGN_TP_CM || id.pgn == keelwire_core::isotp::PGN_TP_DT {
            let (m, _) = self.rx_tp.push(now, id.priority, id.pgn, id.source, id.destination, f.payload());
            if let Some(m) = m {
                return self.handle_message(&N2kMessage { timestamp_us: now, priority: m.priority, pgn: m.pgn, source: m.source, destination: m.destination, data: m.data });
            }
            return vec![];
        }
        if self.rx_fp.is_fast(id.pgn) {
            if let Some(a) = self.rx_fp.push(now, id.priority, id.pgn, id.source, id.destination, f.payload()) {
                return self.handle_message(&N2kMessage { timestamp_us: now, priority: id.priority, pgn: id.pgn, source: id.source, destination: id.destination, data: a.data });
            }
            return vec![];
        }
        self.handle_message(&N2kMessage { timestamp_us: now, priority: id.priority, pgn: id.pgn, source: id.source, destination: id.destination, data: f.payload().to_vec() })
    }
}

fn bank_payload(instance: u8, st: &[u8; 28]) -> Vec<u8> {
    let mut d = vec![instance, 0, 0, 0, 0, 0, 0, 0];
    for (k, s) in st.iter().enumerate() {
        set_bits(&mut d, 8 + k * 2, 2, *s as u64);
    }
    d
}

impl CanInterface for Simulator {
    fn name(&self) -> String {
        if self.conflict {
            "Simulator (instance conflict)".into()
        } else {
            "Simulator".into()
        }
    }

    fn caps(&self) -> InterfaceCaps {
        InterfaceCaps { raw_frames: true, can_transmit: true, ..Default::default() }
    }

    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, mut outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut banks = HashMap::new();
            let mut clmd = [3u8; 28];
            for (k, s) in clmd.iter_mut().enumerate().take(16) {
                *s = if [0, 2, 3, 7, 10].contains(&k) { 1 } else { 0 };
            }
            banks.insert(0, clmd);
            let mut ckm = [3u8; 28];
            for s in ckm.iter_mut().take(12) {
                *s = 0;
            }
            banks.insert(1, ckm);
            let mut st = State {
                devs: devices(self.conflict),
                seq: SequenceCounter::default(),
                banks,
                tick: 0,
                rng: 0x2545_F491_4F6C_DD1D,
                rx_fp: Reassembler::default(),
                rx_tp: TpTracker::default(),
            };
            let _ = events.send(BusEvent::Status(InterfaceStatus::Connected { detail: format!("{} simulated devices", st.devs.len()) })).await;
            // Initial address claims, staggered.
            for i in 0..st.devs.len() {
                for f in st.claim(i) {
                    let _ = events.send(BusEvent::Frame(f)).await;
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs_f64(0.1 / self.rate));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        st.tick += 1;
                        let mut frames = Vec::new();
                        for i in 0..st.devs.len() {
                            frames.extend(st.periodic(i));
                        }
                        for f in frames {
                            if events.send(BusEvent::Frame(f)).await.is_err() { return; }
                        }
                    }
                    o = outbound.recv() => {
                        let Some(o) = o else { return };
                        let replies = match o {
                            Outbound::Frame(f) => st.handle_frame(&f),
                            Outbound::Message(m) => st.handle_message(&m),
                        };
                        // Small, realistic response delay.
                        if !replies.is_empty() {
                            tokio::time::sleep(Duration::from_millis(3)).await;
                        }
                        for mut f in replies {
                            f.timestamp_us = clock::now_us();
                            if events.send(BusEvent::Frame(f)).await.is_err() { return; }
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(conflict: bool) -> State {
        State { devs: devices(conflict), seq: SequenceCounter::default(), banks: HashMap::new(), tick: 0, rng: 1, rx_fp: Reassembler::default(), rx_tp: TpTracker::default() }
    }

    #[test]
    fn answers_product_request() {
        let mut s = state(false);
        let req = N2kMessage::new(6, iso::PGN_ISO_REQUEST, 0x80, 10, iso::iso_request(iso::PGN_PRODUCT_INFO));
        let frames = s.handle_message(&req);
        assert!(frames.len() > 10);
        let mut r = Reassembler::default();
        let mut got = None;
        for f in &frames {
            let id = f.id();
            // 126996 is a PDU2 (broadcast-format) PGN, so the reply cannot carry a destination.
            assert_eq!(id.source, 10);
            if let Some(a) = r.push(0, id.priority, id.pgn, id.source, id.destination, f.payload()) {
                got = Some(a);
            }
        }
        let p = ProductInfo::parse(&got.unwrap().data).unwrap();
        assert_eq!(p.model_id, "GPS200");
    }

    #[test]
    fn switch_control_updates_status() {
        let mut s = state(false);
        let ctl = N2kMessage::new(3, 127502, 0x80, 255, iso::switch_bank_control(0, &[(5, true)]));
        let frames = s.handle_message(&ctl);
        assert_eq!(frames.len(), 1);
        let (inst, st) = iso::parse_switch_bank(frames[0].payload()).unwrap();
        assert_eq!(inst, 0);
        assert_eq!(st[4], 1);
    }

    #[test]
    fn standard_field_commands() {
        let mut s = state(false);
        // DST110 (address 13): depth offset -0.3 m (128267 field 3, signed 1 mm)
        let cmd = GroupFunction::Command { pgn: 128267, priority: 15, params: vec![(3, (-300i16).to_le_bytes().to_vec())] }.encode();
        s.handle_message(&N2kMessage::new(3, 126208, 0x80, 13, cmd));
        assert_eq!(s.devs[3].depth_offset_mm, -300);
        // temperature source on instance 0 (130316: select field 2, set field 3)
        let cmd = GroupFunction::Command { pgn: 130316, priority: 15, params: vec![(2, vec![0]), (3, vec![1])] }.encode();
        s.handle_message(&N2kMessage::new(3, 126208, 0x80, 13, cmd));
        assert_eq!(s.devs[3].temp_source, 1);
        // TLM100 (address 16): fluid type 1 (fresh water)
        let cmd = GroupFunction::Command { pgn: 127505, priority: 15, params: vec![(1, vec![0]), (2, vec![1])] }.encode();
        s.handle_message(&N2kMessage::new(3, 126208, 0x80, 16, cmd));
        assert_eq!(s.devs[6].fluid, 1);
    }

    #[test]
    fn maretron_compass_and_tank_capacity() {
        let mut s = state(false);
        let req = maretron::command_payload(2686, 1, maretron::command::COMPASS_ROT_CONFIG, &[2, 0, 0xDC, 0x05]);
        let frames = s.handle_message(&N2kMessage::new(4, 126208, 0x80, 11, req));
        assert_eq!(s.devs[1].rot_ms, 1500);
        assert!(frames.iter().any(|f| f.id().pgn == 126720));
        let cmd = GroupFunction::Command { pgn: 127505, priority: 15, params: vec![(4, 4500u32.to_le_bytes().to_vec())] }.encode();
        assert_eq!(cmd, vec![1, 0x11, 0xF2, 0x01, 0xFF, 1, 4, 0x94, 0x11, 0, 0]);
        s.handle_message(&N2kMessage::new(3, 126208, 0x80, 16, cmd));
        assert_eq!(s.devs[6].capacity_l, 450.0);
    }

    #[test]
    fn instance_command_acks() {
        let mut s = state(false);
        let cmd = GroupFunction::Command { pgn: iso::PGN_ADDRESS_CLAIM, priority: 8, params: vec![(3, vec![2])] }.encode();
        let frames = s.handle_message(&N2kMessage::new(3, 126208, 0x80, 11, cmd));
        assert_eq!(s.devs[1].name.device_instance(), 2);
        assert!(frames.iter().any(|f| f.id().pgn == iso::PGN_ADDRESS_CLAIM));
    }
}
