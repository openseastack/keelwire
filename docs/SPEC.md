# Keelwire: Technical System Specification

**Revision:** 2.8 (2026-10-27) — supersedes rev 1.0. Changes are summarized in §21. Rev 2.1 recorded implementation decisions; rev 2.2 corrects the Maretron transport (126208-wrapped requests) and adds community-sourced device commands (Appendix A); rev 2.3 adds the item-by-item feature checklist (Appendix C), units, standard field settings and packaging; rev 2.4 redesigns the interface (§13); rev 2.5 adds the network map and traffic tables (§13.3) and the Keelwire gateway and power-tee hardware (Appendix D); rev 2.6 names the project Keelwire and builds the map and tables; rev 2.7 designs service sessions and reports (§11.5); rev 2.8 builds them and starts the iPhone/iPad app (`docs/IOS-SPEC.md`).

## 1. Executive Summary & Vision

**Keelwire** is an open-source, cross-platform NMEA 2000 (CAN bus) diagnostic, packet analysis, and hardware configuration platform. It gives technicians a single tool for mixed-vendor networks on any computer, with a high-performance **Rust** engine and a modern **web-based interface**.

The system operates in two execution modes:
1. **Desktop Application:** Bundled via Tauri v2 for macOS (Apple Silicon & Intel), Linux, and Windows.
2. **Headless Marine Gateway Daemon:** Executed as a background daemon (`keelwire --headless`) on single-board computers (Raspberry Pi, CM4, Minisforum), serving the web interface over HTTP/WebSocket to any browser on the vessel's local network.

### 1.1 Core Functional Targets

| Feature | Keelwire section |
|---|---|
| Device list with all ISO/NMEA identity fields (address, manufacturer, model, serial, versions, instances, LEN, cert level, product code, DB version) | §6, §12.2 |
| Per-device Transmit / Receive PGN lists | §6.4 |
| Human-readable view of messages transmitted by each device | §12.1 |
| Edit installation descriptions, device/system/data instances, labels, source | §7 |
| PGN enable/disable and transmit-interval configuration | §7.3 |
| Instance-conflict analysis | §10.4 |
| Device configuration for Maretron/Carling devices (Get / Put / Load file / Save file, changed fields highlighted) | §8 |
| Boat files (save/load full network state, offline analysis) | §11.1 |
| Device configuration files (copy config between identical devices/boats) | §11.2 |
| Firmware update of Maretron devices | §8.6 (deferred, see §19) |
| USB100 / IPG100 gateway support | §4.2 |

The item-by-item feature checklist is in **Appendix C**.

### 1.2 Non-Goals (v1)
* Firmware update (protocol and image format are proprietary; tracked as a stretch goal).
* Acting as a certified NMEA 2000 device (we implement the network-management subset needed to transmit safely, not certification).
* Cloud services, user accounts.

---

## 2. High-Level Architecture

```text
┌────────────────────────────────────────────────────────────────────────┐
│                   Web Frontend (React / Vite / Tailwind)               │
│  Sniffer · Devices · Device Config · Switching · Diagnostics · Settings│
└───────────────────────────────────▲────────────────────────────────────┘
                                    │ WebSocket + REST (also inside Tauri)
┌───────────────────────────────────┴────────────────────────────────────┐
│                       Keelwire Core (Rust)                       │
├────────────────────────────────────────────────────────────────────────┤
│ 1. Engine (Tokio)                                                      │
│    - Ingest task → Ring buffer → Reassembly → Decode → Fan-out         │
│    - Device Registry (keyed by ISO NAME, not source address)           │
│    - Metrics, Diff cache, Instance analyzer, Capture writers           │
├────────────────────────────────────────────────────────────────────────┤
│ 2. Network Management (NEW)                                            │
│    - Own ISO Address Claim + contention (60928)                        │
│    - ISO Request (59904) / ISO Ack (59392) / Commanded Address (65240) │
│    - Group Function (126208) client · Heartbeat (126993) tracking      │
│    - TX Guard (read-only by default, explicit arm)                     │
├────────────────────────────────────────────────────────────────────────┤
│ 3. Dissection & Configuration                                          │
│    - Fast-Packet reassembler + fragmenter                              │
│    - J1939 Transport Protocol (BAM + RTS/CTS) (60416 / 60160)          │
│    - CANboat schema decoder (canboat.json)                             │
│    - Standard config engine (126208 Request/Command/Read/Write fields) │
│    - Proprietary schema registry (Maretron 126720 / 61184 / 65280…)    │
│    - Device models: CLMD12, CLMD16, CKM12, VMM6, TLM/TLA, …            │
├────────────────────────────────────────────────────────────────────────┤
│ 4. Hardware Abstraction Layer                                          │
│    SocketCAN · Actisense NGT-1 (BST) · Actisense/YD ASCII RAW          │
│    Maretron USB100 · Maretron IPG100 · Yacht Devices TCP/UDP           │
│    Signal K (canboat JSON) · Replay (pcap/pcapng/candump/canboat/YD)   │
│    Simulator · Loopback                                                │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Technology Stack

* **Backend / Core Engine:** Rust (edition 2021, MSRV 1.80)
  * **Async Runtime:** `tokio` (multi-threaded)
  * **Web / API Server:** `axum` 0.7, `tower-http` (CORS, tracing), `rust-embed` (UI assets compiled into the binary)
  * **CAN & Serial:** `socketcan` (Linux only, behind `cfg(target_os = "linux")`), `tokio-serial`
  * **Serialization:** `serde`, `serde_json`; hand-written bit codecs for N2K fields (bit-level, little-endian, non-byte-aligned fields make `deku` a poor fit for the generic decoder; `deku` remains optional for fixed proprietary structs)
  * **State & Concurrency:** `dashmap`, `parking_lot`, `tokio::sync::{broadcast, mpsc, watch}`
  * **CLI & Logging:** `clap` v4, `tracing`, `tracing-subscriber`
  * **Packet Logging:** in-crate PCAP/PCAPNG writer (`LINKTYPE_CAN_SOCKETCAN` = 227)
* **Desktop Wrapper:** Tauri v2. The Tauri process embeds the same Axum server on `127.0.0.1:<ephemeral>` and points its WebView at it, so desktop and headless share one code path.
* **Frontend:** TypeScript, React 18, Vite 5, Tailwind CSS 3, `lucide-react`, `@tanstack/react-virtual` (row virtualization), `zustand`, `d3-force` (topology).

---

## 4. Hardware Abstraction Layer (HAL)

### 4.1 Interface Trait

Some gateways deliver **raw CAN frames** (SocketCAN, YD RAW, USB100/IPG100 raw mode), others deliver **already-reassembled N2K messages** (Actisense NGT-1 BST 0x93, Signal K). The HAL therefore emits a `BusEvent` enum rather than frames only.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCanFrame {
    pub timestamp_us: u64,   // monotonic µs since engine start (host clock if HW lacks one)
    pub can_id: u32,         // 29-bit identifier
    pub is_extended: bool,
    pub dlc: u8,
    pub data: [u8; 8],
}

#[derive(Clone, Debug)]
pub struct N2kMessage {      // a complete (reassembled) PGN payload
    pub timestamp_us: u64,
    pub priority: u8,
    pub pgn: u32,
    pub source: u8,
    pub destination: u8,
    pub data: Vec<u8>,       // up to 223 B (fast-packet) or 1785 B (ISO TP)
}

pub enum BusEvent {
    Frame(RawCanFrame),
    Message(N2kMessage),
    ErrorFrame { timestamp_us: u64, kind: BusErrorKind },
    Status(InterfaceStatus),   // connected / disconnected / reconnecting / error text
}

pub struct InterfaceCaps {
    pub raw_frames: bool,      // false ⇒ only Message events; sniffer shows synthetic frames
    pub can_transmit: bool,
    pub error_frames: bool,
    pub hw_timestamps: bool,
}

pub struct InterfaceCaps { raw_frames, can_transmit, tx_messages, error_frames, hw_timestamps }

/// Each driver runs as a Tokio task. `spawn` consumes the driver; it emits
/// BusEvents and consumes Outbound traffic until the outbound channel closes or
/// the task is aborted (InterfaceHandle::stop / drop).
pub trait CanInterface: Send {
    fn name(&self) -> String;
    fn caps(&self) -> InterfaceCaps;
    fn spawn(self: Box<Self>, events: mpsc::Sender<BusEvent>, outbound: mpsc::Receiver<Outbound>) -> JoinHandle<()>;
}
```

Drivers that only accept whole messages on TX (NGT-1) receive `Outbound::Message`; frame-level drivers get the message fragmented by the engine (fast-packet / TP). Drivers reconnect with exponential back-off (0.5 s → 10 s) and report `Status` events.

### 4.2 Supported Drivers

Interface URIs are used by CLI and the Settings page:

| URI | Driver | Notes |
|---|---|---|
| `socketcan:can0` | SocketCAN | Linux only. 250 kbit/s configured externally (`ip link set can0 type can bitrate 250000`). Error frames enabled via `CAN_RAW_ERR_FILTER`. |
| `actisense:/dev/ttyUSB0[?baud=115200]` | Actisense NGT-1 BST binary | Frame: `DLE STX cmd len data… chk DLE ETX`, DLE (0x10) stuffed by doubling; checksum makes `(cmd+len+Σdata+chk) mod 256 == 0`. RX cmd `0x93` = N2K message (prio, PGN LE24, dst, src, timestamp LE32 ms, len, data). TX cmd `0x94` (prio, PGN, dst, len, data). The NGT-1 forwards only enabled PGNs; on connect the driver sends the "receive all" PGN-enable command (verify against canboat `actisense-serial`). |
| `actisense-ascii:tcp://host:port` | Actisense N2K ASCII / RAW ASCII (NGX-1, W2K-1) | Line formats `A…` (N2K ASCII) and RAW. |
| `ydraw:tcp://host:1457`, `ydraw:udp://0.0.0.0:1456` | Yacht Devices RAW (YDWG-02, YDNR-02, YDEN-02) | Line format `hh:mm:ss.sss R 19F51323 01 2F 30 70 00 2F 30 70`; TX by writing `19F51323 01 2F…\r\n`. |
| `usb100:/dev/ttyUSB0` | Maretron USB100 | Wire protocol not yet documented; driver reports this clearly. |
| `ipg100:tcp://host[:6543]?password=` | Maretron IPG100 | Implemented (Appendix A). Port 6543 = bus 0, 6553 = bus 1. Delivers reassembled messages; the IPG owns the bus address. |
| `signalk:ws://host:3000` | Signal K | Signal K's standard stream carries decoded deltas, **not** raw N2K. The driver consumes canboat-format JSON (`{prio,pgn,src,dst,timestamp,fields}` or `{…,"data":"hex"}`) emitted by the companion Signal K plugin `signalk-keelwire-tap` (ships in `tools/`), and re-encodes it. Read-only by default. |
| `replay:path?speed=1.0&loop=false` | Replay | Formats auto-detected: pcap / pcapng (linktype 227), candump `-L` log, canboat plain (`ts,prio,pgn,src,dst,len,hex…`), YD RAW log, Actisense N2K ASCII. |
| `sim:` | Simulator | Synthetic vessel (see §17.1). Responds to ISO Request, 126208, 127502. |
| `loopback:` | Loopback | TX frames are echoed back; used by tests. |

---

## 5. Protocol Dissection Engine

### 5.1 29-bit CAN ID Decomposition

$$\text{CAN ID} = (\text{Priority} \ll 26) \mid (\text{EDP} \ll 25) \mid (\text{DP} \ll 24) \mid (\text{PF} \ll 16) \mid (\text{PS} \ll 8) \mid \text{SA}$$

```rust
pub struct CanId { pub priority: u8, pub pgn: u32, pub destination: u8, pub source: u8 }
impl CanId { pub fn decode(id: u32) -> Self; pub fn encode(&self) -> u32; }
```
* **PDU1 (PF < 240):** PGN = `(EDP<<17)|(DP<<16)|(PF<<8)`, PS = destination address.
* **PDU2 (PF ≥ 240):** PGN = `(EDP<<17)|(DP<<16)|(PF<<8)|PS`, destination = 255.
* `encode` must clear PS from the PGN for PDU1 PGNs and insert the destination.

### 5.2 Fast-Packet (`core::fastpacket`)
* **Frame 0:** byte 0 = `seq<<5 | 0`, byte 1 = total length N (≤ 223), bytes 2–7 = data[0..6].
* **Frame k (1…31):** byte 0 = `seq<<5 | k`, bytes 1–7 = next 7 data bytes. Unused trailing bytes are `0xFF`.
* **Reassembler:** sessions keyed by `(source, pgn, seq)`; a frame 0 for an existing key restarts it; frames received for unknown sessions are counted as `fp_orphan`; out-of-order frames are accepted into a bitmap; complete when all `ceil((N-6)/7)+1` frames arrive. Stale sessions (> 750 ms since last frame; NMEA allows ≤ 750 ms gaps) are dropped and counted as `fp_timeout`.
* **Fragmenter (NEW):** for TX; maintains a 3-bit sequence counter per `(pgn)`.
* Which PGNs are fast-packet is taken from the CANboat database (`"Type": "Fast"`), with a built-in fallback list for ISO/proprietary ranges: `126208, 126464, 126720, 126996, 126998` and `130816–131071`.

### 5.3 ISO 11783-3 / J1939 Transport Protocol (`core::isotp`) (NEW)
* TP.CM = PGN 60416 (0xEC00), TP.DT = PGN 60160 (0xEB00). Payload up to 1785 bytes.
* Control bytes: 16 RTS, 17 CTS, 19 EndOfMsgAck, 32 BAM, 255 Abort.
* **Receive:** BAM (broadcast) and RTS/CTS (destination-specific to *our* address: we answer CTS for all packets, then EndOfMsgAck). Timeouts: T1 = 750 ms, T2 = T3 = 1250 ms, T4 = 1050 ms.
* **Transmit:** BAM with 50 ms inter-packet gap (used for 65240 Commanded Address). RTS/CTS transmit is optional.
* Monitors TP sessions between *other* nodes (passive sniffing) so the sniffer can show reassembled ISO TP payloads.

### 5.4 CANboat Schema Decoder (`core::canboat`)
* **Source:** `canboat.json` (canboat/canboat `docs/canboat.json`), embedded at build time (gzip) and overridable at runtime with `--pgn-db path`.
* **Lookup:** `PGN → Vec<PgnDefinition>` (several definitions per PGN for proprietary PGNs, disambiguated by `Match` fields such as Manufacturer Code = 137 / Industry Code = 4 / proprietary ID).
* **Field types:** NUMBER (signed/unsigned, resolution, offset, unit), FLOAT, DECIMAL (BCD), LOOKUP, INDIRECT_LOOKUP, BITLOOKUP, FIELDTYPE_LOOKUP, TIME, DATE, MMSI, STRING_FIX, STRING_LZ, STRING_LAU (len+encoding, ASCII/UTF-16), BINARY, RESERVED/SPARE, PGN, ISO_NAME, VARIABLE (126208 parameters), DYNAMIC_FIELD_*.
* **Special values:** all-ones = "not available", all-ones−1 = "out of range"/reserved, all-ones−2 = "error" (for unsigned ≥ 2 bit fields) → decoded as `null` with a `status` flag.
* **Repeating fields:** `RepeatingFieldSet1Size/Count/StartField` (and set 2) supported.
* **Output:** `DecodedMessage { pgn, name, description, fields: Vec<(id, name, Value, unit)> }` plus a one-line summary for the sniffer.

---

## 6. Network Management (NEW)

### 6.1 Device Identity
* A device is identified by its 64-bit ISO **NAME**, never by source address (addresses move when devices are added). The registry maps `NAME ↔ current address` and records address-change history.
* NAME layout (little-endian 64 bit): Unique Number (21) · Manufacturer Code (11) · Device Instance Lower (3) · Device Instance Upper (5) · Device Function (8) · Reserved (1) · Device Class (7) · System Instance (4) · Industry Group (3) · Arbitrary Address Capable (1).
* Device Instance = `upper<<3 | lower`.

### 6.2 Own Address Claim
* Before transmitting anything, the engine claims an address with PGN 60928 (preferred address configurable, default 0x80 / 128; scan upward on conflict, `254` = cannot claim).
* Default NAME: Manufacturer Code configurable (default 2046, unassigned/open-source placeholder), Industry 4 (Marine), Device Class 25 (Inter/Intranetwork Device), Function 130 (PC Gateway), Arbitrary Address Capable = 1, Unique Number derived from host ID.
* Contention: on receiving a claim for our address, compare NAMEs — lower NAME wins; if we lose, re-claim the next free address. Respond to ISO Request for 60928 with our claim. Wait 250 ms after claiming before other TX.
* Our node answers ISO Requests for 126996 (Product Info: "Keelwire", version) and 126464 (TX/RX lists), and sends 126993 Heartbeat every 60 s.

### 6.3 Discovery
* On start (and on "Refresh"), send ISO Request (59904) for 60928 to global (255).
* For every newly seen NAME: request (addressed) 126996, 126998, 126464 (TX + RX list via 126208 Request, field 1 = 0/1), with retry ×3, 500 ms spacing, never more than 4 outstanding requests per destination (bus courtesy).
* Track 126993 heartbeats; a device is **offline** after `max(3 × heartbeat interval, 60 s)` without any frame.
* **Duplicate address detection:** two different NAMEs transmitting from the same source address within 2 s (or a non-claim frame from an address whose claimant just lost arbitration) raise an alert.

### 6.4 Device Properties Captured
From 60928: all NAME fields. From 126996: N2K database version, product code, model ID, software version, model version (hardware), serial code, certification level, LEN. From 126998: Installation Description 1 & 2, Manufacturer Information. From 126464: transmit & receive PGN lists. Maretron-only (Appendix A): boot loader version, labels, channel table. Observed: PGNs actually transmitted with rate (Hz) and per-device bandwidth share.

---

## 7. Standard Configuration Engine (NEW — works on all vendors)

All via PGN **126208 NMEA Group Function** (fast-packet, addressed to the device), answered by 126208 **Acknowledge** (function 2).

### 7.1 Group Function Encoding
| Function | Code | Layout after function code |
|---|---|---|
| Request | 0 | PGN (24) · Transmission interval ms (32, 0xFFFFFFFF = unchanged) · Interval offset ×10 ms (16, 0xFFFF = unchanged) · N pairs (8) · pairs |
| Command | 1 | PGN (24) · Priority (4, 8 = unchanged) · Reserved (4) · N pairs (8) · pairs |
| Acknowledge | 2 | PGN (24) · PGN error (4) · Interval/priority error (4) · N params (8) · param error codes (4 each) |
| Read Fields | 3 | PGN (24) · [Mfr code (11) · Rsvd (2) · Industry (3) if proprietary] · Unique ID (8) · N selection pairs (8) · N params (8) · selection pairs · field numbers |
| Read Reply | 4 | as Read with values |
| Write Fields | 5 | PGN · [proprietary header] · Unique ID · N selection pairs · N params · selection pairs · (field, value) pairs |
| Write Reply | 6 | as Write |

A **pair** = field number (8, 1-based in the target PGN definition) + value whose width is that field's bit length from the CANboat database (rounded up to whole bytes; strings encoded as STRING_LAU).

### 7.2 Operations
| Operation | Implementation |
|---|---|
| Change device instance | Command to PGN 60928, field 3 (Device Instance Lower) and field 4 (Device Instance Upper) — or Maretron proprietary path if the device NAKs |
| Change system instance | Command to PGN 60928, field 8 |
| Installation description 1 / 2 | Command to PGN 126998, field 1 / 2 (STRING_LAU, ≤ 70 chars) |
| Data instance of a data PGN (e.g. 127505 tank, 127508 battery) | Command to that PGN with selection pair = current instance field, value = new instance |
| PGN transmit enable/disable/rate | Request to the PGN with transmission interval (0 = disable per NMEA convention, 0xFFFFFFFE = restore default) |
| Change source address | 65240 Commanded Address via ISO TP BAM (NAME + new address) |
| Switch control | 127502 Switch Bank Control, or Command on 127501 indicator fields |
* Every write is followed by a read-back (ISO Request) to confirm and update the UI diff state.
* Ack error codes are mapped to readable messages (e.g. "PGN not supported", "Access denied", "Request or command parameter out of range").

---

## 8. Proprietary Device Configuration Engine

### 8.1 Proprietary PGN Families (corrected)
| PGN | Type | Addressing |
|---|---|---|
| 61184 (0xEF00) | Proprietary single frame | Addressed (PDU1) |
| 65280–65535 (0xFF00–0xFFFF) | Proprietary single frame | Broadcast |
| 126720 (0x1EF00) | Proprietary **fast-packet** | **Addressed** (PDU1 — PS is the destination) |
| 130816–131071 | Proprietary fast-packet | Broadcast |

All start with the proprietary header: **Manufacturer Code (11 bits) · Reserved (2 bits, set to 1) · Industry Code (3 bits)**. For Maretron: code 137, industry 4 → first two payload bytes `0x89 0x98`. Carling Technologies (OctoPlex, current owner of Maretron) has its own manufacturer code; the registry accepts both.

### 8.2 Schema Registry
The command-ID values and payload layouts are **not** documented publicly. They are captured as data, not code:

```text
crates/keelwire-core/schemas/proprietary/maretron/*.json
```
```json
{
  "manufacturer": 137, "industry": 4, "pgn": 126720,
  "message": "clmd16.breaker_config",
  "match": { "command_id": "0x????" },
  "verified": false,            // true only after round-trip on real hardware
  "source": "capture xyz.pcapng",
  "fields": [ {"name": "channel", "bits": 8, "type": "uint"}, … ]
}
```
* The codec is generic (bit-packed fields, same primitives as §5.4), so new layouts are added without code changes.
* Unverified schemas are **read/decode only**; writes require `verified: true` *or* the user explicitly enabling "experimental writes" for that device.

### 8.3 Common Configuration Operations
* **Get Config From Device** · **Put Config To Device** · **Load Config From File** · **Save Config To File** · **Restore Factory Defaults** · **Close (warn on unsaved)**.
* Fields that differ from the device's current configuration are rendered in red.
* "User Config Allowed" (per-breaker lock on CLMD12) is respected: locked breaker fields are read-only.

### 8.4 Device Models

#### A. CLMD12 (12-channel Solid-State Power Controller)
* Device: Label, Instance (read-only = device instance), Input Voltage (12/24/36 V/Undefined), status flags Not Configured, Serial Number Invalid.
* Per breaker (×12): Label, State (RO), Default State (OFF/ON/LAST), Default Lock (LOCKED/UNLOCKED), Sound Alarm When Tripped, Manual Mode, Trip Delay (0–750 ms), Inrush Delay (0–1500 ms), Default Dim (5–100 %), Current Dim (RO), Dimming Allowed, Factory Current Rating, Current Rating, Current/Voltage (RO), Flash Map (0 = none, 1–15), Combine With (paralleling, ≤ 180 %), User Config Allowed. Status bits: Short Load, Locked, Tripped, ON, Open Load, Comms Error, Communicating.
* Flash maps (×15): Cycles (0 = ∞, 1–255), On Time / Off Time (0–6553.5 s, 0.1 s), Start in Off, Finish in On.
* Discrete I/O (×36): Data Instance, Indicator, Manufacturer Code (2047 = any), Function (Always ON, Always OFF, Toggle, Brighten, Dim, Flash, OFF w/Lock, Unlock, One-Button Smooth High→Low, One-Button Smooth Low→High, Smooth Scroll, One-Button Smooth Scroll, Discrete Momentary), Assigned Breakers (bitmask).
* Inputs (×7, reported as 127501 indicators 13–19): Label, OnLevel (High/Low/Both), State (RO); Transmit Battery Voltage + Battery Instance (127508).
* Advanced: Device Instance, Transmit 127501, Receive 127501 via 126208, Transmit 127508, Send Extended Status (on error / on request), 126208 receive filter (Carling/Maretron only / any). Box status RO: current, temperature, voltage.

#### B. CLMD16 (16-channel ECB with switching logic engine)
* Per breaker (×16): Label, Manual Mode, State (OFF/ON/TRIP), PWM Default, Default State, Default Lock, **Type** (DC, PWM, Soft-Start DC, Full-Bridge, Soft-Start Full-Bridge — bridge pairs 1&2, 11&12), **Current Rating**, **Instantaneous Pickup** (5–20 × rating), **Short-Time Pickup** (1.5–6.0 × rating, 0.1 step), **Long-Time Delay** (FAST 5 s / MEDIUM 10 s / SLOW 20 s, i²t model), Input Signal, Toggle Mode, PWM Counter. Status: Tripped, Instant/Short/Long-Time Trip, Thermal Protection, Load Shed, HW Fault, Short-to-Ground, Short-to-Battery, Overload, Open Circuit, Locked.
* **Signal enumeration** (inputs to every logic element): None; Input 1–12 Low/Float/High; Network Input 1–16; Logic Output 1–48; Latch 1–16; Toggle 1–16; Timer 1–16; Flash 1–16; Counter Active 1–16; Over-Current Fault 1–16; Tripped 1–16; Thermal Limit 1–16; Discrete I/O 1–32; Breaker On 1–16.
* Logic elements: 48 × 3-input LUT (A, B, C, 8-bit truth table); 16 Latches (Set, Reset); 16 Toggles; 16 Timers (signal, No Delay/OFF Delay/ON Delay, time); 16 Flash (On, Off, Cycles, Enable); 16 Counters (Inc/Dec/Reset signal, type Active High/Low/Rising/Falling/Change/One-Button Smooth Scroll, Min/Max set point, Press/Hold step, Hold periods, Active threshold); 32 Discrete I/O (Data Instance, Indicator); Alarm Enable signal.
* Grouping: 4 groups (1&2, 3&4, 7&8, 13&14) parallel or independent.
* Inputs (×12): Label, OnLevel (Low/High), thresholds & hysteresis (V for 1–8, Ω for 9–10, mA for 11).
* Tanks: 2 resistive + 1 current-loop channel (Enable, Label, Pressure @4/20 mA, Capacity, Tank Number 0–15, Type, Damping 0.2–25 s, 2–16 point calibration).
* Load shedding: per-channel priority (HIGH/MEDIUM/LOW) + 4 voltage thresholds.
* Advanced RO: Location Address, Model Version, temperatures (12A G1-3, 25A, MCU), supply V + OV/UV faults, loop timing, over-current fault, run time, commissioned time.
* The UI shows the logic engine as an editable node graph (§13).

#### C. CKM12 / VMM6 (keypads)
* CKM12: per key (×12) per LED (×3): **Data Instance + Indicator** whose 127501 state lights the LED. Key status is broadcast in 127501 with data instance = device instance. (No RGB/flash-mode configuration exists — rev 1.0 was wrong.)
* VMM6: per switch label/instance/indicator mapping (Appendix A).

#### D. Tank adapters — TLM100 / TLM150 / TLM200 / TLA100 / FPM100 (tank mode)
* Label, Tank Number (0–15), Fluid Type (Fuel, Fresh Water, Waste Water, Live Well, Oil, Black Water, Reserved 1–7), Capacity, Damping, Installation offsets.
* Calibration: **Manual Table** (2–16 points, level-measure → %) or **Step Fill** wizard (start empty, add volume steps, Complete stores table). Rev 1.0's "polynomial" is a piecewise-linear table.

#### E. Additional families (decode first, configure later)
ACM100, ALM100, CLM100, DCM100, DCR100, DST110, EMS100, FFM100, FPM100, GPS100/200, IPG100, J2K100, NBE100, RAA100, RIM100, SIM100, SMS100, SSC200/300, TMP100, USB100, WSO100. Most of their common settings (instance, damping, PGN enable/rate, installation description) work through §7 without any proprietary schema.

### 8.5 Labels
Maretron labels (device, channel) are proprietary; the label table is read for every Maretron device during discovery so the Device List and channel views can show them.

### 8.6 Firmware Update (deferred)
Out of scope for v1. If the protocol is recovered, it will be implemented as a guarded, resumable state machine with image checksum verification and a hard "do not power off" UI.

---

## 9. Switching & Control (NEW detail)
* Status from **127501 Binary Switch Bank Status** (instance + 28 × 2-bit indicators) and Maretron proprietary breaker status (current, voltage, trip flags; Appendix A).
* Control via **127502 Switch Bank Control** (instance + 28 × 2-bit, 3 = no change) by default, or 126208 Command on 127501 (CLMD "Receive 127501 via 126208").
* Dimming via the vendor-proprietary command (Appendix A) or NMEA 2000 v3 switching PGNs when supported.

### 9.1 Safety Model
Three tiers:
1. **Passive** (`--passive`): nothing is ever transmitted.
2. **Query** (default): network management only — own address claim, ISO Requests and 126208 *Requests* for identification (126996, 126998, 126464), heartbeat, answering requests addressed to us. Never changes another device's state.
3. **Armed**: state-changing traffic (126208 Commands, interval changes, 65240, 127502, proprietary writes). Requires the server flag `--allow-tx` **and** arming in the UI; auto-expires after 15 minutes without a control command.

Gateways that own their bus address (NGT-1, IPG100) report `tx_messages`; for them the engine skips its own address claim (`gateway_managed`).
* Configuration writes and switch commands require a confirmation dialog listing the exact frames to be sent (hex) when "expert mode" is on.
* `POST /api/v1/bus/tx` (raw injection) is disabled unless armed and `expert` is enabled.
* Every TX is written to an audit log (`audit.jsonl`: time, user/IP, device NAME, operation, payload).

---

## 10. Diagnostic & Packet Sniffing Engine

### 10.1 Ring Buffer
Fixed-capacity circular buffer (default 500,000 frames ≈ 24 B each), with monotonically increasing sequence numbers so clients can page by `seq` without races. Supports pause/freeze (the UI view stops scrolling while ingest continues).

### 10.2 Differential Byte Highlighter
Per `(pgn, source, instance-ish key)` cache of last payload. Each byte gets `delta ∈ {-1,0,+1}` and `changed_at`. UI renders green (increase) / red (decrease) fading over 2 s. Keyed by message (not frame) for fast-packet PGNs.

### 10.3 Metrics
* Frames/s, messages/s, bus load % (= Σ frame bits / 250,000 per second; extended frame bits = 67 + 8·DLC plus estimated stuffing ≈ +10 %), peak load (1 s windows over last 60 s), error frames/s, fast-packet orphans/timeouts, TP aborts, dropped (channel overflow) count, per-device load share, per-PGN rate.

### 10.4 Instance Conflict Analyzer (NEW)
Rules, each producing a finding with severity and suggested fix:
1. Same PGN + same data instance (+ same source/type field where the PGN has one, e.g. tank fluid type, temperature source) transmitted by **different** devices → **conflict** ("bouncing needle").
2. Two devices with identical (Manufacturer, Device Function, Device Class, Device Instance) → **warning**.
3. Duplicate source address between NAMEs (§6.3) → **error**.
4. Device not responding to ISO Request / missing 126996 → **info**.
5. PGN rate deviates > 50 % from its nominal rate in the CANboat DB → **info**.

### 10.5 Wireshark Integration
* **PCAPNG capture** with interface `LINKTYPE_CAN_SOCKETCAN` (227): 16-byte SocketCAN header (`can_id` big-endian with `CAN_EFF_FLAG` 0x80000000, `len`, pad, data[8]).
* **Extcap** — the same binary implements the extcap contract:
  * `keelwire --extcap-interfaces` → `extcap {version=…}` + `interface {value=keelwire}{display=Keelwire}`
  * `--extcap-dlts --extcap-interface keelwire` → `dlt {number=227}{name=CAN_SOCKETCAN}{display=SocketCAN}`
  * `--extcap-config --extcap-interface keelwire` → `arg {number=0}{call=--source}{display=Source URI}{type=string}`
  * `--capture --extcap-interface keelwire --fifo PATH --source URI` → writes classic pcap to the FIFO. When `--source` is `ws://host:8080`, it streams from a running Keelwire instance instead of opening hardware (non-exclusive).
  * Installer note: copy/symlink the binary into Wireshark's `extcap/` folder.

---

## 11. Files & Persistence (NEW)

### 11.1 Boat File (`*.keelwire`, JSON, gzip optional)
```json
{ "format": "keelwire-boat", "version": 1, "saved_at": "...", "vessel": "...",
  "devices": [ { "name_hex": "...", "address": 12, "identity": {...}, "product": {...},
                 "config_info": {...}, "pgn_lists": {...}, "labels": {...},
                 "observed_pgns": [...], "proprietary_config": {...} } ],
  "messages_snapshot": [ /* last decoded message per (pgn,src) */ ] }
```
* Loading a boat file puts the app in **offline mode**: device list, properties, transmitted-PGN view, config dialogs (read-only) and instance analyzer all work.
* Import of other tools' boat files: not planned.

### 11.2 Device Config File (`*.kwcfg`)
`{ "format": "keelwire-device-config", "model": "CLMD16", "manufacturer": 137, "firmware": "...", "config": {...} }`. Loading into a different model is refused; loading into a different firmware version warns.

### 11.3 Captures
pcapng (default), candump log, canboat plain. Settings: rotate by size (default 100 MB) or time; output dir configurable.

### 11.4 Settings
`~/.config/keelwire/settings.json` (platform-appropriate dir): last interface, own NAME/address, UI prefs. Headless: `--config path`.

### 11.5 Service sessions and reports (rev 2.7; built in rev 2.8)
Mockups: the "Service sessions and reports" row of the design canvas.
* **Session.** Starts automatically when a gateway connects and records the whole session to pcapng (other vessels' AIS kept in the capture, optionally stripped). A **disconnect pauses** the session and it resumes on reconnect, so a cable swap stays one visit; the pause is logged. After 30 minutes away, or when Keelwire restarts, the next connect asks whether to continue the open session or start a new one. Only *End session* closes it.
* **Controls.** A red **REC button** at the top right of every screen (pulsing dot and elapsed time; amber when paused by a disconnect, grey when paused by the technician, *Start session* when none is open) opens a menu: pause/resume, add a note, open the session page, end and save. A **Session** item in the left menu opens the session page: boat details (name, MMSI and call sign from AIS, read-only; optional HIN), visit details (technician name, remembered per computer; start time; GPS position; gateway; pauses), the timeline and notes.
* **Vessel identity.** From the boat's own AIS transponder: reports whose *AIS Transceiver information* is a VDL *transmission* (129038/129039, values 2–3) or the transponder's own static data (129794 Class A; 129809/129810 Class B) give MMSI, name, call sign, IMO (Class A), ship type and dimensions. The technician confirms it. Without AIS: optional boat name and optional hull ID (HIN); otherwise the session is filed under its date. Shown at the top of every screen (name, MMSI, call sign, session timer) and on every report with date, time, technician and the boat's GPS position during the service (from its own 129025/129029, first fix of the session).
* **Recorded.** Device snapshots at start and end; a **change log** of every command sent (time, device, address, serial, setting, before → after, method such as 126208 Command or Write, and whether the device confirmed it); findings at start and end (fixed / still open); traffic statistics; technician notes.
* **Outputs** (chosen when ending): owner report (PDF, plain language); technician report (PDF or HTML: change log, findings, since-last-visit, bus statistics, file list with SHA-256); session bundle (.zip: both reports, boat files before and after, capture, changes.csv, devices.csv, pgns.csv). No business branding. Reports cover only this boat; other vessels' AIS is omitted.
* **Headless (Raspberry Pi).** Sessions are saved on the Pi and listed on its web interface, where reports and bundles download to the phone or laptop viewing it. Target hardware includes the Pi Zero 2W (the engine uses about 36 MB of RAM and 1–2 % CPU on simulated traffic). Bus connection options are under evaluation: USB to a Keelwire Gateway (power and data on one cable), or a CAN HAT on the bus directly.
* **Implementation (rev 2.8).** `keelwire-server::session` (vessel identity, AIS tracker, change log, history, since-last-visit diff, AIS strip), `::report` (finalize, CSV, PDF and HTML reports), `::pdf` (a small built-in PDF writer embedding Geist, so no PDF library or browser is needed, including on a headless Pi). The change log is an API middleware: every device-changing request (`PATCH /devices/:k/standard`, `POST /devices/:k/{fields,data-instance,pgn-rate,address,tank-capacity,maretron,maretron/action}`, `PUT /devices/:k/config`, `POST /switch`, `POST /bus/tx`) is recorded with the device before and after, the method, and whether the device confirmed it. The open session lives in `<data dir>/session-open/` (session.json saved every 30 s and on every change; pcapng chunks `capture-NNN.pcapng`, one per recording stretch) and is reopened after a restart. Simulator: an AIS transponder (Vesper XB-8000, MMSI 367550123 "SECOND WIND") plus three nearby ships. Options: `--sessions-dir` (`KEELWIRE_SESSIONS_DIR`), `--no-auto-session`. Re-render a saved session's reports: `cargo run -p keelwire-server --example render_report -- <session folder>`.
* **History.** Sessions are filed per vessel (MMSI, else HIN, else name) under `Documents/Keelwire/Vessels/`. On connect, a known boat is recognised and the Overview shows what changed since the last visit (devices added or gone, software versions, settings).

---

## 12. Web API & Real-Time Protocol

### 12.1 WebSocket (`/ws`)
Client → server JSON:
```json
{"op":"subscribe","topic":"packets","filter":"pgn == 127501 && src == 0x05","decoded":true}
{"op":"subscribe","topic":"devices"}   {"op":"subscribe","topic":"metrics"}
{"op":"subscribe","topic":"alerts"}    {"op":"unsubscribe","topic":"packets"}
```
Server → client, batched at ≤ 60 Hz:
```json
{"topic":"packets","items":[{"seq":1,"ts":123456,"prio":2,"pgn":127250,"src":5,"dst":255,
  "data":"ff7c…","diff":[0,1,-1,…],"name":"Vessel Heading","summary":"Heading 123.4°…","fields":[…]}],
 "dropped":0}
{"topic":"devices","items":[{…device…}]}   {"topic":"metrics","data":{…}}
{"topic":"alerts","items":[{"kind":"address_conflict","severity":"error","message":"…"}]}
```
Backpressure: if a client's queue exceeds 5,000 items it is sent a `dropped` count and items are discarded (never block the engine).

### 12.2 REST (`/api/v1`)
| Method & path | Purpose |
|---|---|
| `GET /status` | version, interface, caps, own address/NAME, armed, offline-mode |
| `GET /interfaces` · `POST /interface` `{uri}` · `DELETE /interface` | list / connect / disconnect |
| `POST /tx/arm` `{armed, expert}` | TX guard |
| `GET /devices` · `GET /devices/:name` | registry |
| `POST /devices/refresh` | re-run discovery |
| `GET /devices/:name/pgns` | observed + declared TX/RX PGN lists with rates |
| `GET /devices/:name/messages` | last decoded message per PGN ("Transmitted PGNs window") |
| `PATCH /devices/:name/standard` `{device_instance?, system_instance?, install1?, install2?}` | §7 |
| `POST /devices/:name/pgn-rate` `{pgn, interval_ms}` | §7 |
| `POST /devices/:name/data-instance` `{pgn, current, new}` | §7 |
| `POST /devices/:name/address` `{address}` | 65240 |
| `GET /devices/:name/config` · `PUT /devices/:name/config` · `POST /devices/:name/config/defaults` | proprietary (§8) |
| `POST /switch` `{instance, indicator, state}` · `POST /switch/dim` `{…}` | §9 |
| `POST /bus/tx` `{can_id, data}` or `{pgn, prio, dst, data}` | raw (expert) |
| `GET /packets?after_seq=&limit=` | ring buffer paging |
| `POST /capture/start` `{format, path?}` · `POST /capture/stop` · `GET /captures` · `GET /captures/:file` | |
| `GET /boatfile` (download) · `POST /boatfile` (upload → offline mode) · `DELETE /boatfile` (back online) | §11 |
| `GET /analysis/instances` | §10.4 |
| `GET /pgn-db` · `GET /pgn-db/:pgn` | PGN reference |
| `GET /metrics` | snapshot |
| `GET /session` · `PATCH /session` `{name?, hin?, technician?, confirm?, manual?}` | open session (state, vessel, timeline, counts) / edit boat and visit details (§11.5) |
| `POST /session/start` · `/pause` · `/resume` · `/new` | start; pause or resume recording; save the open session and start a new one |
| `POST /session/notes` `{text}` · `DELETE /session/notes/:ts` | notes |
| `GET /session/since-last` | what changed on this boat since its last saved session |
| `POST /session/end` `{owner_pdf, technician: "pdf"\|"html"\|null, bundle, strip_other_ais}` | file the session, write reports and bundle → `{dir, rel, files}` |
| `GET /history` · `GET /history/file?path=<boat>/<session>/<file>[&inline=1]` | boats and their sessions; download a saved file |

Errors use `{ "error": "code", "message": "…" }` with proper status codes. Headless mode binds `0.0.0.0:8080` by default with optional `--token` (Bearer / `?token=`). The plain binary in desktop use binds `127.0.0.1:8080` and opens the default browser (`--no-browser` to suppress). `--port N` (`KEELWIRE_PORT`) or `--bind ADDR` choose the port; an explicit port that is busy is an error, while the default 8080 falls through to the next free port (8081…8099) with a warning. The Tauri wrapper binds `127.0.0.1` on an ephemeral port.

**Packaging.** One self-contained executable per platform contains the engine, API and web UI (embedded with rust-embed). `cargo build` compiles the UI through `build.rs` (Node.js needed only on the build machine; `KEELWIRE_SKIP_UI_BUILD=1` skips it). Themes: Day, Dark (follows the OS dark-mode preference on first start), Dusk, Night (red).

---

## 13. Frontend Application Specification

Rev 2.4 redesign. Principles, from user research on networking tools (UniFi, Sniffnet, Grafana reviews): lead with what needs doing, not with statistics; plain language first, protocol detail one click away; changes unlocked where they are made; charts that show gaps and stay legible in dark modes. Visual system: Geist Sans / Geist Mono, cool neutral ground, one blue accent; Day, Dark and Night (red) themes; SVG chart kit (`ui/src/lib/charts.tsx`: time area chart with gaps and hover, sparkline, bar list, stacked bars, share bar). Works down to phone width (bottom tab bar).

### 13.1 Views
1. **Overview** (home) — headline ("10 devices online. One thing to fix."), *Needs attention* with one-click fixes from the analyzer (set a free data instance, set a device instance, ask to identify, "It's intentional" to dismiss deliberate redundancy, *Show devices* highlights them), KPI tiles (devices, bus load with sparkline, errors, backbone power from LEN), bus-load chart over 1 min / 10 min / 1 h with grey gaps while no gateway was connected (server keeps one hour, `/api/v1/metrics/history`), busiest devices, *On the bus now* live readings grouped by system, recent events, bus details. Replaces the Diagnostics page.
2. **Devices** — *By system* list (navigation, engine and power, tanks, switching, displays and gateways) with a live reading and message-rate sparkline per device; *Table* (identity and traffic columns, chooser, sort, inline edit, CSV, print) and *Map* views. A wide device panel replaces the drawer and the Configure dialog: **Overview** (big live values, history chart of the main reading, setup summary, identity), **Settings** (plain-language sections: name and location, readings, instances, how often it reports; edits are staged and highlighted with their previous value, one *Apply* sends them all and reports what each device confirmed; *Advanced* holds Maretron settings, every message field, address and settings files), **Messages** (Transmitted PGNs per instance), **Identity** (all properties, transmit/receive lists, channels).
3. **Traffic** — *Latest per message* (one line per message type and sender, updated in place, with rate and count) or *Every frame* (the virtualized log); stacked messages-per-second timeline by sender; filter chips (from device, hide network management) plus the expression language; Record to pcapng. Detail panel: decoded fields with byte ranges, colour-coded byte map with changed-byte outlines, live history of the first numeric field, copy as candump.
4. **Switching** — summary (total load, circuits on, tripped, share of load by circuit), toggle switches per circuit with current bars, keypads shown as read-only key indicators; *Unlock switching* in place.
5. **Connect** — first-run and *Change connection* screen: detected USB gateways (serial port scan), IPG100 address form, *Only listen*, and simulator, boat file, replay, Yacht Devices, SocketCAN and Signal K options.
6. **Settings** — connection address, sending to the bus (listen only / identify, expert mode), captures, boat files, display theme, units, about.

### 13.2 Global UI
Left rail (bottom bar on phones) with labels: Overview, Devices, Traffic, Switching, Settings; theme switch. Top bar: connection chip (opens Connect), global search (`/`; devices, or a PGN number to filter Traffic), recording indicator, **Changes locked/unlocked** button. While unlocked an orange banner shows a live countdown and *Lock now*. The server's `--allow-tx` and listen-only state are explained when unlocking is refused.

### 13.3 Network map and traffic tables (built in rev 2.6)
Devices gets a **Table | Map** switch. Mockups: the "Keelwire Network Map & Device Table" design canvas.

* **Map.** A horizontal backbone with a terminator at each end; each device is a card on a drop. Layout switch **Alternating** (cards above and below) or **One side** (wraps into rows to fit the window); phones show a vertical backbone. Drop thickness follows the device's traffic; solid = online, dashed grey = not responding, amber = needs attention (with **Fix**). The gateway in use is marked *You're here*.
  * Order is never guessed. Until arranged, cards are grouped by device class (from the address claim) under a banner "Order unknown". **Arrange backbone** lets the technician drag cards and place terminators and the power tee. Rev 2.6 saves the layout on the computer (browser storage); saving it in the boat file is next. Positions are labelled *set by technician* or *measured* (Appendix D).
  * **Card fields:** name, type and status always; the user picks up to 3 more (address and messages/s, live reading, share of bus, instance, LEN, software, serial).
  * **Bus health strip** above the map: termination, bus errors, bus load, declared load (LEN), voltage at gateway; with an Keelwire gateway also gateway temperature and Wi-Fi signal, and with a power tee its input voltage and current. Values a gateway cannot supply read "Not measured" / "Not reported", never zero.
* **Table, by device:** status, name, maker and type, address, msgs/s, share of bus, messages, data, PGNs sent, incomplete multi-frame messages, last heard, LEN, software; optional serial, instances, hardware version, product code/model ID, NMEA 2000 version and certification level, installation description, frames/s, first seen, address changes. Column chooser (remembered), sort, filter chips (online, not responding, needs attention, type, maker), search (name, `@address`, serial, PGN number or name), CSV export. Expanding a row lists every PGN the device sends with rate, count, last received and latest decoded value, linking to Traffic.
* **Table, by PGN:** one row per PGN with senders, rate, share of bus (by frames), messages, frames per message, last, latest value; chips for several senders, multi-frame, stopped, needs attention.
* **Server:** cumulative counters per sender and per (PGN, sender) since a resettable start: messages, frames, bytes, frames per message, first and last seen, incomplete fast-packets per sender (`keelwire-core::traffic`); `GET /api/v1/stats`, `POST /api/v1/stats/reset`.

---

## 14. Security
* Headless default bind `0.0.0.0:8080`; recommend `--token` on shared vessel networks. CORS restricted to same origin unless `--cors-any`.
* TX guard (§9.1). Audit log of all transmissions.
* No telemetry.

---

## 15. Performance Requirements
* Sustain 2,000 frames/s ingest (≈ 100 % load at 250 kbit/s is ~1,850 frames/s) with < 10 % CPU on a Raspberry Pi 4 for the engine.
* End-to-end latency frame → UI < 100 ms.
* Memory: ring buffer + registry < 64 MB at default settings.

---

## 16. Repository & Project Directory Structure

```text
keelwire/
├── Cargo.toml                    # workspace
├── crates/
│   ├── keelwire-core/              # pure protocol logic, no I/O
│   │   ├── schemas/              # canboat.json (+ proprietary/*.json)
│   │   └── src/{frame,fastpacket,isotp,canboat,iso,groupfunction,registry,
│   │            sniffer,metrics,analysis,filter,pcap,boatfile,proprietary/}
│   ├── keelwire-hal/               # drivers: socketcan, actisense, ascii (YD/Actisense), maretron,
│   │                             #          signalk, replay, sim, loopback
│   ├── keelwire-server/            # engine, network management, axum API, CLI binary `keelwire`
│   └── keelwire-tauri/             # desktop wrapper (not in default-members)
├── ui/                           # React app (built into ui/dist, embedded by keelwire-server)
├── tools/
│   ├── n2k_sim_send.py           # Python test sender (YD RAW over UDP/TCP, candump)
│   └── signalk-keelwire-tap/       # Signal K plugin forwarding raw/canboat JSON
├── docs/SPEC.md
└── tests/                        # integration tests (replay, loopback, round-trips)
```

---

## 17. Testing Strategy (NEW)

### 17.1 Simulator (`sim:`)
Synthetic vessel: GPS (129025/129026/129029), heading (127250), wind (130306), depth (128267), engine (127488/127489), battery (127508), tanks (127505), 2 × switch banks (127501) that obey 127502, and devices with address claims, 126996, 126998, 126993 heartbeat, 126464 lists. Optional fault injection: duplicate instance, address collision, fast-packet loss.

### 17.2 Test Layers
* Unit: CAN ID round-trip, fast-packet reassembly (in-order, out-of-order, timeout, sequence reuse), TP BAM/RTS-CTS, NAME bitfields, 126208 encode/decode, CANboat decode against reference vectors (129025 position, 127250 heading, 130306 wind, 126996 strings), filter parser, pcap writer (re-read by parser).
* Round-trip: every proprietary schema `encode(decode(x)) == x`.
* Integration: loopback interface — claim address, request, configure, verify ack; replay golden capture and compare decoded output snapshot.
* Hardware-in-the-loop (manual checklist): USB100 & IPG100 connect, discovery, instance change on a test device, CLMD read-config.

### 17.3 Python Sender (`tools/n2k_sim_send.py`)
Std-lib only; sends a scripted set of PGNs (incl. fast-packet) as YD RAW over UDP/TCP or writes candump logs; useful with Signal K or a YDWG to inject traffic.

---

## 18. Implementation Roadmap

### Phase 0: Protocol Documentation (parallel)
* Collect proprietary command IDs, per-model configuration layouts and gateway transports from public community documentation (CANboat and others) and from captures of traffic on hardware the project owns.
* Record results as schema JSON (§8.2) + Appendix A; mark `verified:false` until confirmed with captures on hardware.

### Phase 1: HAL & Base Identification
* `RawCanFrame`, `BusEvent`, `CanId` encode/decode; SocketCAN, Actisense BST, YD RAW, replay, sim, loopback.
* CLI: `keelwire dump --interface <uri> [--decode]`.

### Phase 2: Assembly, Schema & Network Management
* Fast-packet reassembler/fragmenter, ISO TP, CANboat decoder, registry keyed by NAME, discovery, own address claim, TX guard.

### Phase 3: Configuration
* 126208 engine (§7) with read-back; proprietary schema registry + Maretron header + device models (§8); round-trip tests; USB100/IPG100 drivers once Appendix A is filled.

### Phase 4: API, Sniffer & Frontend
* Axum REST + WS (60 Hz batching, backpressure); React views (§13); instance analyzer; boat/config files; captures.

### Phase 5: Packaging & Extcap
* Extcap contract; Tauri v2 (macOS universal: `aarch64-apple-darwin` + `x86_64-apple-darwin`, Windows MSI, Linux deb/rpm/AppImage); systemd unit for headless; GitHub Actions CI matrix.

---

## 19. Open Items & Risks
| # | Item | Mitigation |
|---|---|---|
| 1 | Maretron proprietary layouts unknown | Community documentation + capture-based verification; unverified schemas are read-only |
| 2 | USB100 transport protocol unknown | Same as #1; until then use SocketCAN / YD / Actisense hardware |
| 3 | Writing wrong config to breakers can de-energise critical loads | TX guard, confirmations, verified-schema requirement, audit log |
| 4 | NGT-1 delivers reassembled messages only | `BusEvent::Message`; sniffer shows synthesized frames, flagged |
| 5 | Manufacturer code for our own NAME | Configurable; request an NMEA code if the project is published |
| 6 | Proprietary device layouts | Use only public community documentation and the project's own captures; keep schemas factual (field layouts); ship no vendor code or binaries |
| 7 | Firmware update | Deferred |
| 8 | Keelwire hardware: isolation, current rating, certification | Appendix D.8 |

---

## 20. Appendix A — Maretron / Carling Proprietary Protocol

Sources (all open, credited in NOTICE): CANboat (Apache-2.0), mbj4668/n2k (MIT, Maretron definitions observed on the wire), Maretron's public SSC300 user's manual, signalk-tlm100-config (MIT, confirmed on a TLM100), signalk-n2k-switching (MIT).

### A.1 How Maretron configuration travels (implemented)
* **Request:** PGN 126208 *Command* addressed to the device, commanded PGN 126720, priority byte 0xF8, parameter count 4, then `01 89 98` (Maretron header), `02 <product code LE16>`, `03 <software code LE16, 1>`, then the Maretron **command byte** and its data with no further index byte.
* **Reply:** PGN 126720 "slave response" `89 98 · product code · software code · command · data`, and/or a 126208 Acknowledge for PGN 126720. The engine listens for both before sending.
* The CANboat opcode table (0 Read All … 87 Write Channel Config Extended) is kept as reference data but is **not** what devices have been observed to accept.

### A.2 Commands (implemented as named actions)
| Device | Command | Data | Reply | Status |
|---|---|---|---|---|
| SSC200/SSC300 | 93 variation source | `FF` = read, 0–3 = set | source · variation (0.0001 rad) | read confirmed by traces |
| SSC200/SSC300 | 94 rate of turn / installation | 16-bit sub-command: 1 get damping, 2 set damping (+ms LE16), 17 start zeroing, 18 cancel, 513 get installation, 514 set (+orientation LE16 +direction LE16) | sub-command · values | gets confirmed by traces; sets **unconfirmed** (sources disagree on 8- vs 16-bit sub-command) → expert mode |
| SSC200/SSC300 | 36 installation offset | heading ×10 LE16 | ack | **unconfirmed** (upstream notes a mismatch with the manual) |
| SSC200/SSC300 | 80 resend calibration status | — | status | confirmed |
| SSC200/SSC300 | 240 reset and start deviation calibration | — | status messages | confirmed |
| TLM100 (product 27725) | 49 get level configuration | unknown | reserved · row · depth (u32, units unknown) · level (0.004 %) | reply layout from traces; request data **unknown** |
| TLM100 | 33 set level configuration | unknown | — | not implemented |

Supplementary decode definitions for these replies live in `crates/keelwire-core/schemas/community.json` (CANboat JSON format, merged into the database at load).

### A.3 Standard-PGN configuration that works on Maretron devices (implemented)
* **Tank capacity:** 126208 Command, PGN 127505, priority byte 0xFF, one pair: field 4 = capacity in 0.1 L (LE32). Confirmed on a TLM100 (350 → 450 L). Read-back by watching the next 127505.
* **Switching:** Maretron/Carling banks may ignore 127502. The engine also sends a 126208 Command on PGN 127501 addressed to the bank's device: pair 1 = bank instance, pair n+1 = indicator n (0/1).

### A.4 Carling digital switching (decode only, from CANboat; manufacturer code 176)
61184 Breaker Command (bitmaps for breakers 1–19, command, dim value); 126720 DC Configuration Command (inrush delay, trip delay, configuration flags, current rating, factory max rating, load-shed priorities A/B, dim value, breaker group, flash map index — matches the CLMD12 settings); 65300 Switchboard Status and 130921 Breaker Status and Configuration (envelopes only).

### A.5 Still unknown
| Item | Notes |
|---|---|
| CLMD16 breaker, logic-engine and I/O configuration | not in any open source found |
| CLMD12 configuration under Maretron code 137 (vs Carling 176) | Carling 126720 layout may apply; needs a capture |
| CKM12 / VMM6 key-LED mapping | |
| TLM100 calibration-table request data and depth units | |
| Label write | read via 130818 works |
| USB100 serial protocol | |
| `.net` boat-file format, firmware update | out of scope v1 |

Recommended way to fill gaps: record a pcapng capture with Keelwire (IPG100 or SocketCAN) while changing one setting at a time with the vendor tool, then add a schema JSON with `read`/`write` command specs and keep `verified: false` until a read → write → read round trip matches.

## Appendix B — Reference Layouts
* **ISO Request 59904:** PGN (24 bit LE). PDU1, PF = 0xEA.
* **ISO Ack 59392:** Control (0 ACK, 1 NAK, 2 Access Denied, 3 Cannot Respond) · Group Function · 3 × 0xFF · PGN (24).
* **Address Claim 60928:** 8-byte NAME (§6.1). PF = 0xEE, destination 255.
* **Commanded Address 65240:** NAME (64) + new address (8), sent via TP BAM.
* **Heartbeat 126993:** interval (16, ms) · sequence counter (8) · controller states.
* **PGN List 126464:** function (0 = Transmit, 1 = Receive) · PGN (24) × n.
* **Switch Bank Status 127501 / Control 127502:** instance (8) · 28 × 2-bit (0 off, 1 on, 2 error, 3 unavailable / no-change).

---

## Appendix C — Feature Checklist

What a technician expects from an NMEA 2000 analysis tool, checked against the implementation on 2026-10-27.
✔ implemented and tested against the simulator · ◐ partly · ✗ not implemented (reason given).
"Standard" means it works with any manufacturer through NMEA 2000 messages; "proprietary" items need manufacturer layouts not yet published (see Appendix A.5 for the capture procedure).

### C.1 Application

| Feature | Keelwire | Status |
|---|---|---|
| Open and save a boat file (network state, configuration, last messages) | Settings → Boat files; `.keelwire` file association (desktop app); `--open FILE` or a bare path on the command line | ✔ |
| Print and export the device list | Devices → Print (print stylesheet) and CSV export | ✔ |
| Connect / disconnect a gateway | Settings → Bus interface; first-run Connect screen | ✔ |
| Refresh device information | Devices → Refresh; new senders are queried automatically on first appearance; offline after 60 s of silence | ✔ |
| Gateway choice and USB auto-detection | IPG100, NGT-1, YD RAW, Actisense ASCII, SocketCAN, Signal K, replay, simulator; **Find USB gateways** | ◐ USB100 protocol still unknown |
| Display units | Settings → Units: depth, distance, speed, temperature, volume, pressure; Metric / US presets | ✔ |
| Listen only | Settings → Sending to the bus → Listen only (`--passive`) | ✔ |
| Column chooser and sorting | Devices → Table → Columns (grouped, remembered) and click-to-sort | ✔ |
| Received PGNs with field descriptions | Device → Identity → Receive list; click a PGN for its fields (bits, units, lookup list, description from CANboat) | ✔ |
| Transmitted PGNs, live, one entry per instance | Device → Messages: one line per message and per instance, expand/collapse, all fields + raw bytes, age, Clear | ✔ |
| Device properties | Device → Identity: every NAME, product and configuration field, transmit and receive lists | ✔ |
| Instancing analysis with recommendations | Continuous analysis; conflicts listed with specific advice (which device to change, lowest free instance, redundancy note); **Show devices** highlights them; one-click fix where a standard command exists | ✔ |
| Edit installation descriptions | Inline in the device table, or the device's Settings (126208 Write on 126998) | ✔ |
| Edit device and system instance | Inline, or the device's Settings (126208 Command on 60928) | ✔ |
| Restore factory defaults | No standard command exists; manufacturer commands need captures | ✗ |
| Firmware update | Out of scope for v1 (§8.6) | ✗ |
| Help | README, docs/INSTALL.md, docs/SPEC.md; Settings → About | ◐ no in-app help yet |
| Service sessions | Starts on connect, REC button on every screen, pauses on disconnect, Session page (boat, technician, timeline, notes) | ✔ |
| Boat identified from its own AIS | Transceiver "transmission" flag in 129038/129039, static data 129794/129809/129810; confirm dialog; typed name/HIN without AIS | ✔ |
| Change log and reports | Every change with before/after and read-back; owner report (PDF), technician report (PDF or HTML), session bundle (.zip with pcapng, boat files, CSVs) | ✔ |
| History per boat and since-last-visit | Boats page; Welcome-back banner on the Overview | ✔ |

### C.2 Device list

| Behaviour | Status |
|---|---|
| Hardware channels per device | ✔ Device → Identity → Channels (HW channel, indicator, source, instance, label) |
| Identity columns: address, manufacturer, model, hardware and software version, serial, device and system instance, installation descriptions, manufacturer information, class, function, NMEA 2000 version, certification level, product code, LEN, unique number | ✔ (most hidden by default; enable with Columns) |
| Source type (fluid / temperature source) | ✔ derived from the device's data messages |
| Data instances | ✔ derived from the device's data messages (all instances listed) |
| Channel labels | ◐ when the device broadcasts them; proprietary labels ✗ |
| Traffic columns: messages/s, frames/s, share of bus, messages, data, PGNs sent, incomplete multi-frame messages, first seen, last heard | ✔ (§13.3) |
| Own gateway highlighted, offline devices dimmed, devices needing attention marked | ✔ |
| Inline editing | ✔ device/system instance and installation descriptions (when changes are unlocked); data instance and label via the device's Data fields |

### C.3 Device configuration — common controls

Load / Save configuration file ✔ · Get / Put ✔ · changed fields highlighted ✔ · Restore Factory Defaults ✗ (see C.1) ·
PGN enable/disable and interval ✔ standard 126208 · installation descriptions ✔ · device instance ✔ ·
**Data fields** ✔: any setting held in a standard message field (data instances, temperature/humidity/pressure source, fluid type, tank capacity, depth transducer offset, battery configuration 127513) sent as a 126208 Command (selecting the channel by its key fields on multi-channel devices) and confirmed from the device's next broadcast.

### C.4 Maretron device support

| Device | Covered | Not yet (proprietary unless noted) |
|---|---|---|
| ACM100 | instance, PGN rates, descriptions | label, circuit/device type, CT install & range, damping, energy reset |
| ALM100 | descriptions, device instance | label, instance, annunciator test |
| CKM12 | keypad state via 127501 | key ↔ LED mapping, data instance / indicator |
| CLM100 | instance via data fields where broadcast | function, 4/20 mA scaling, damping, label |
| CLMD12 | switching (127502 + 126208), currents | breaker settings, flash map, discrete I/O, inputs, advanced — **capture pending (hardware available)** |
| CLMD16 | switching, currents, trip status | logic engine (counters, flash, grouping, inputs, tanks, latch, load shedding, logic, timer, toggle, discrete I/O, advanced) |
| DCM100 | instance; battery type/capacity/chemistry/coefficients via standard 127513 *if the device broadcasts it* | DC type, current-sensor zero offset |
| DCR100, RIM100, SIM100 | instances, PGN rates | counters/timers, indicator mapping |
| DSM150/200/250/410/570 | installation descriptions | display configuration |
| DST110 | depth transducer offset (128267, standard), sea temperature instance/source (130312/130316, standard) | speed calibration, temperature offset, trip-log reset |
| EMS100, FFM100, FPM100 | instances, PGN rates | sensor setup, flow calibration, tank tables |
| GPS100 / GPS200 | PGN rates, instance | SBAS, GPS mode, cold start, masks, antenna altitude |
| IPG100 | — (configured from its own web page) | gateway settings |
| J2K100 | engine instance (data fields) | J1939 mapping |
| NBE100 | — | PGN filter |
| RAA100 | instance | rudder angle calibration |
| SMS100 | — | phone numbers, alerts, test |
| SSC200 / SSC300 | variation source (read/set), rate-of-turn damping (read), installation orientation (read), deviation calibration start & status (Appendix A) | some writes unconfirmed (expert); NMEA 0183 settings |
| TLA100 / TLM100 / TLM150 / TLM200 | capacity (hardware-confirmed encoding), fluid type, instance (standard) | level calibration tables |
| TMP100 | per-channel instance and source (standard) — unconfirmed on hardware | label, damping |
| USB100 | — | gateway protocol (§4.2) |
| VMM6 | — | key configuration |
| WSO100 | instance, PGN rates | installation offset, damping |

### C.5 Beyond the checklist

Traffic sniffer with filter language and byte-diff colouring; pcapng/candump/CANboat capture and replay; Wireshark extcap and live pcap stream; network map and traffic tables (§13.3); bus-load history, error and fast-packet counters; any-vendor gateways (SocketCAN, NGT-1, YD, Signal K) and a multi-platform single binary with web UI; headless Raspberry Pi mode; Day/Dark/Night themes; phone layout.

## Appendix D — Keelwire Hardware: Gateway and Power Tee (rev 2.5, proposed)

A technician's pack of two ESP32 units that give the analyzer measurements no commercial gateway reports. The analyzer works without them; with them, the bus-health values in §13.3 become real measurements. Owner decisions (rev 3 handoff): two gateway boards from one firmware image: the **Keelwire Gateway** (rev A; logger and Wi-Fi bridge; caps `err,temp,volt_coarse,tee`) and the **Keelwire Tech Gateway** (USB-isolated, panel Micro-C; caps `err,temp,volt,term,pulse,tee`). Measurement is off by default and runs only in a technician session. The app enables features from `caps` only, holds one TLS WebSocket per app instance (gateway limit: 2 clients), and does all analysis; the gateway keeps a <80 Ω interlock on the terminator command. Power Tee v1: Micro-C, 4 A, passive power path; the switchable terminator stays in the tee and needs physical confirmation (on rev A, plus a technician-entered meter reading). The complete, current hardware brief is `docs/HARDWARE-HANDOFF.md`; where this appendix and the handoff differ, the handoff wins.

### D.1 Units
| Unit | Where it goes | What it adds |
|---|---|---|
| **Keelwire Gateway** | Any drop (a normal NMEA 2000 device) | Wi-Fi gateway for the app; CAN error counters; termination measurement; its own supply voltage; cable resistance back to power; temperature and Wi-Fi signal |
| **Keelwire Power Tee** | Replaces the boat's power-insertion tee | Total bus current and the current to each side, insertion voltage, one software-switched terminator |

Walking a probe from tee to tee to map the bus was considered and **is not a default workflow**: it takes too long for the value. The two units above give most of the benefit with no walking. One optional spot check remains: move the gateway to the farthest drop and press **Measure here** to get the far-end voltage (D.5).

### D.2 Gateway
* ESP32 with TWAI (CAN) controller and a 3.3 V CAN transceiver; bus-powered (NET-S/NET-C), declared LEN to cover its own draw plus the test pulse.
* **Wi-Fi:** own access point by default, or joins the boat's network. Streams frames in a format the HAL already reads (Yacht Devices RAW over TCP/UDP), plus a JSON status channel for measurements.
* **CAN errors:** TWAI transmit/receive error counters, bus-off and the type of each bus error (bit, stuff, CRC, form, ACK), reported as counts per minute.
* **Termination:** a current-limited differential test source (≈0.2 V through ~1 kΩ) across CAN-H/CAN-L, sampled by a fast sample-and-hold ADC triggered from the CAN RX line only during inter-frame idle (≥ 11 recessive bits, ~44 µs). Classified as ≈60 / ≈120 / ≈40 Ω / open, with the app correcting for transceiver loading by node count. Bench proof of concept required first.
* **Supply and cable:** NET-S voltage through a divider and precision ADC. A MOSFET-switched known load (100–250 mA) pulsed in a fixed pattern (e.g. 50 ms on/off, averaged over many cycles) gives the loop resistance back to the supply: R = ΔV / ΔI.
* **Housekeeping:** ESP32 temperature, Wi-Fi RSSI, channel, reconnects, uptime, firmware version.

### D.3 Power Tee
* Fused battery input, two backbone ports (A and B), CAN-H/CAN-L passed straight through.
* **One current sensor per side** (shunt + INA228-class monitor; v1 Micro-C, 4 A). Nothing active in the power path (no reverse-polarity MOSFET), so power keeps flowing if the electronics fail. Insertion voltage measured on the same chip.
* Input TVS; electronics supplied through a PTC and a buck converter.
* **One software terminator:** CAN-H/CAN-L pass straight through the tee, so a single 120 Ω across them serves whichever side is the backbone end. Switched by a latching relay or photoMOS solid-state relay that tolerates CAN fault voltages; the state is kept through power loss and reboots (latching relay, or restored from flash before CAN traffic starts). Rules: the app offers *Turn on terminator* only when the measured termination says one is missing **and** the technician physically confirms nothing continues past one side (port current is supporting evidence only); it refuses when enabling would read ≈40 Ω; every change is read back by re-measuring. Changes require the same unlock as other changes.
* No CAN transmit: the tee only listens (optional) and reports over ESP-NOW.

### D.4 Link between the units
* **ESP-NOW** on the gateway's Wi-Fi channel, concurrent with the gateway's access point or station connection (no mode switching). The tee scans for the gateway's channel; pairing from the app (*Pair*) after holding the tee's button; encrypted with an ESP-NOW local master key.
* Messages: `PAIR`, `STATUS` (1 Hz: input V, current A and B, terminator states, temperature, firmware), `PULSE_START/STOP` (pattern, amplitude), `MEASURE` (window results), `TERM_SET` (on/off) and `TERM_STATE`. Pairing: X25519 key exchange confirmed with the 6-digit code on the tee's label (or SPAKE2), so a sniffed pairing can't be brute-forced offline. Link loss is a normal state; the terminator relay latches.
* The gateway forwards everything to the app as hardware events on the existing WebSocket (`hw.gateway`, `hw.tee`).

### D.5 What the app computes
* **Real total current** = A + B, shown beside the declared LEN total; **load split** per side.
* **Which side the gateway is on:** the gateway pulses its load and the tee sees the step on port A or B. No resistance maths needed.
* **Cable resistance tee → gateway:** (ΔV at tee − ΔV at gateway) / ΔI from the same pulse. Measuring both ends removes the supply's own resistance; divided by the cable's resistance per metre (selected cable type) it gives the cable distance.
* **Voltage at the gateway now** and **predicted at full declared load:** V_tee − R × (declared current of that side, or of the whole bus when the split is unknown). Warn below 9 V.
* **Far-end spot check (optional):** with the gateway moved to the farthest drop, the same pulse gives that point's measured voltage and its full-load prediction.
* Positions on the map from these results are labelled *measured*; mismatches with the technician's layout are flagged.

### D.6 Deferred
* **Inline drop probe** (between a tee and one device): that device's real current vs its LEN, and automatic placement on its drop.
* **Time-domain reflectometry** for total length (needs a time-to-digital converter; service mode only).

### D.7 App changes
Connect lists Keelwire gateways and paired tees; the bus-health strip shows their values; the map places the gateway on the side the tee reports; a Power Tee panel shows input voltage, total and per-side current, and the terminator switch. Full firmware/hardware brief: `docs/HARDWARE-HANDOFF.md`.

### D.8 Open items
1. ESP32 variant (TWAI count, Wi-Fi band, temperature sensor) and CAN transceiver.
2. Galvanic isolation of the gateway's CAN interface if it also has a USB port that can connect to shore-powered equipment.
3. Current rating, connectors (Micro-C / Mini-C; likely the largest part of the cost), and the fuse arrangement for the tee.
4. Latching relay vs photoMOS for the terminator (on-resistance, capacitance, fault tolerance).
5. NMEA 2000 certification before using the NMEA 2000 name or logo on the products, particularly the gateway, which transmits.

---

## 21. Change Log from Rev 1.0

**Rev 2.6:** Project named **Keelwire**. Crates `keelwire-*`, binary `keelwire`, bundle id `org.keelwire.app`, boat files `.keelwire`, environment variables `KEELWIRE_*`, mDNS `_keelwire._tcp`, hardware family Keelwire Gateway / Tech Gateway / Power Tee. New icon (a K drawn as a terminated backbone with two drops), `docs/brand/`. Network map and traffic tables built (§13.3); simulator now sends 127506 as fast-packet; product tour in `site/`, published by `.github/workflows/pages.yml`.

**Rev 2.5:** Network map and traffic tables designed (§13.3): backbone map with alternating/one-side layouts, technician-set order, card fields, bus-health strip; device and PGN tables with column chooser, filters, search, per-device PGN breakdown; cumulative statistics endpoints. Overview LEN tile renamed *Declared load (LEN)* and flags devices that did not report LEN. Appendix D: Keelwire gateway and power tee (CAN errors, termination measurement, supply and current measurement, software terminator, ESP-NOW link), revised after hardware review (rev 3: two boards with caps, USB isolation on the Tech Gateway, 100 mA pulse/LEN 4, opt-in measurement, 4 A Micro-C tee, idle-window termination sampling, X25519 pairing, function 137, one TLS session per app). Release pipeline and installers (docs/INSTALL.md, docs/RELEASING.md).

**Rev 2.4:** Interface redesign (§13): Overview home with one-click fixes, device panel with staged settings replacing the Configure dialog, Traffic *Latest per message* view, Switching toggles, first-run Connect screen, unlock-in-place with countdown, Geist type, Day/Dark/Night themes, SVG chart kit, phone layout. Server: one-hour bus-load history with gaps, structured fix suggestions in analysis findings.


**Rev 2.3:** Appendix C feature checklist; device list columns/sort/chooser/print/CSV; Transmitted PGNs per instance with Clear; Received PGNs field descriptions; Units setting; 30-second instancing analysis with specific advice and highlighting; Data fields tab (standard 126208 field commands with read-back); USB gateway finder; boat-file association; single-binary packaging, `--port`, Dark theme.

* Added: functional parity table, network management (address claim, discovery, NAME identity, heartbeat), ISO TP, standard 126208 configuration engine, switching control & safety model, instance analyzer, boat/config files, simulator, loopback, testing strategy, security, performance budget, open items, appendices.
* HAL: introduced `BusEvent` (frames *and* messages), capabilities, reconnection; added USB100, YD RAW, Actisense ASCII drivers; corrected Signal K (standard stream is not raw N2K).
* Corrected proprietary PGN table (126720 is addressed PDU1; added 61184 / 130816+), proprietary header bit layout, fast-packet timeout (750 ms per NMEA, was 500 ms).
* Replaced CLMD12/16, CKM12 and TLM data models with corrected ones (rev 1.0 trip curves, RGB backlight and polynomial tank curves do not exist on these devices).
* Extcap: added `--extcap-dlts`, `--extcap-config`, `--capture --fifo`, remote `ws://` source.
