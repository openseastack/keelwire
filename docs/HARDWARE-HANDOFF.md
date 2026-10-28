# Keelwire hardware family: handoff (rev 3)

**For:** the agent building the ESP32 gateways and the Power Tee.
**From:** the Keelwire app (Rust core; desktop app, headless server, mobile to follow). App spec: `docs/SPEC.md` §13.3 and Appendix D.
**Status:** rev 3 (2026-10-26). It merges rev 2 with the hardware agent's response (`Keelwire-Hardware-Handoff-rev3-response.md`, agreed by the owner) and the app agent's answers (§0). **[must]** = the app relies on it. **[decide]** = open. **[PoC]** = prove on the bench first.

## 0. App agent's answers to the rev 3 questions

1. **`volt_coarse` as its own capability: yes.** The app shows it as "≈12.4 V" (one decimal, marked approximate). It does no cable-resistance or full-load prediction from it. It doesn't subtract it from the tee's voltage either, because ±1–2 % is ±0.25 V, bigger than most cable drops.
2. **At most 2 concurrent HTTPS/WebSocket clients: yes, on one condition.** Each app instance must need only **one** TLS session. So commands go over the WebSocket as well as REST (§4.2): one app instance = one WebSocket, plus short REST calls on the same keep-alive connection only when the socket is down. Two clients then means a technician's laptop and a phone at the same time. The app's own browser and webview front-ends never connect to the gateway directly; they go through the Rust core, which holds the single session. A third client gets `503` with `Retry-After`, and the app shows "Gateway busy: 2 apps connected".
3. **The split: yes.** The ESP32 owns the RAW stream, the logger, the tests (including lock-in demodulation and the raw termination value) and ESP-NOW. The Rust app owns classification, cable resistance, distance, full-load prediction and the map. **One exception, for safety:** the gateway keeps a simple hard interlock for the terminator command (§5.3). It refuses `on: true` when its own latest raw reading is below 80 Ω (would end near 40 Ω), whatever the app says. That's a threshold check, not analysis.

Two more requests from the app side, both new in rev 3:
* **RAW stream transmit (§2.8):** the YD RAW port is plain TCP with no authentication. When transmit is enabled, accept transmit lines only from the client IP address that enabled it in an authenticated session, and turn transmit off when that session ends.
* **Rev A and the tee terminator (§5.3):** rev A can't measure termination, so rule 1 can't come from the gateway. Allow the command on rev A only with a technician-entered meter reading: `"meter_ohm": 118` (measured with the network powered down), which must classify as one missing. The gateway checks the same < 80 Ω interlock on that value.

## Decisions (owner)
1. **Two gateway boards.** The **Keelwire Gateway (rev A)** is the logger and Wi-Fi bridge, ordered now, unchanged. The **Keelwire Tech Gateway** reuses rev A's core and adds a measurement section. One firmware image runs on both.
2. **The Tech Gateway is isolated at the USB port, not at CAN.** Its measurement circuits stay bus-referenced.
3. **The app is a native Rust application** (Windows, Linux, macOS; iOS and Android to follow). No browser talks to a gateway directly.
4. **Power Tee v1:** Micro-C, 4 A, passive power path, latching terminator switched only after the technician confirms the backbone end.

---

## 1. The device family

| Device | Board | `caps` (mDNS TXT and `/api/v1/info`) | Status |
|---|---|---|---|
| **Keelwire Gateway** | rev A, 72 × 40 mm, 4 layers | `err`, `temp`, `volt_coarse`, `tee` | Ordering now |
| **Keelwire Tech Gateway** | derived from rev A | `err`, `temp`, `volt`, `term`, `pulse`, `tee` | Layout after the termination PoC passes |
| **Keelwire Power Tee** | new | none (no IP presence; reached through a gateway over ESP-NOW) | Design now |

**[must]** The app turns each feature on from `caps` only, never from the model name:

| Cap | App shows |
|---|---|
| `err` | Bus errors, error counters, errors by type (if not `null`) |
| `temp` | Gateway temperature |
| `volt_coarse` | "≈12.4 V" at the gateway; no derived values |
| `volt` | Voltage at the gateway (2 decimals) |
| `term` | Termination value and classification; *Measure now* |
| `pulse` | Pulse test: side of the tee, cable resistance, distance, voltage at full load, spot check |
| `tee` | Paired Power Tees: voltage, current per port, terminator |

```
 laptop / phone (Rust app) ── one TLS WebSocket per app instance ──┐
                         └── YD RAW TCP stream ──────────────────┐ │
                                                                 ▼ ▼
 ┌──────────────────────────┐   ESP-NOW (same channel as Wi-Fi)  ┌────────────────┐
 │ Gateway (rev A) or        │◄─────────────────────────────────►│ Keelwire Power Tee│
 │ Tech Gateway              │                                    └──┬──────────┬──┘
 └──────┬───────────────────┘                             battery ───┘          │
        │ drop                                                                   │
 ═══════╧══════════════ NMEA 2000 backbone ══════════════════════ port A ─┤tee├─ port B ═══
```

---

## 2. Gateways

### 2.1 Traffic stream **[must]** (both boards)
**Yacht Devices RAW** (the app's `ydraw:` driver):
* TCP server on a configurable port (optionally also UDP broadcast), advertised by mDNS.
* Received frame, one per line: `hh:mm:ss.sss R 19F51323 01 2F 30 70 00 2F 30 70\r\n`. Frames the gateway itself transmitted use `T`.
* App → bus: `19F51323 01 2F 30 70 00 2F 30 70\r\n`, transmitted unchanged only under the rules in §2.8. Default: listen-only.

### 2.2 Discovery **[must]** (both)
mDNS `_keelwire._tcp` on the API port. TXT: `model` (`gateway` / `tech-gateway`), `hw_rev`, `serial`, `fw`, `api` (`1`), `raw` (RAW port), `tls` (`1`), `caps` (comma-separated, §1).

### 2.3 Rev A hardware the firmware relies on
| Item | Rev A |
|---|---|
| MCU | ESP32-S3-MINI-1-N4R2 (2 MB PSRAM) |
| CAN | TCAN3414 on TWAI: TX GPIO12, RX GPIO13, STB GPIO14 (low = normal) |
| microSD | SDMMC 4-bit: CLK 48, CMD 34, D0 35, D1 36, D2 47, D3 33, card detect 21 |
| Supply sense | GPIO1 (ADC1), divider 100k/10k (ratio 0.0909), ±1–2 % → `volt_coarse`; 220 µF hold-up to close the log on power loss |
| USB | Native USB-C, **not isolated: bench use only** (ties the laptop's ground to NET-C) |
| Connection | Pigtail through an M16 gland to a screw terminal |
| LEN | Estimate 2 (≈60 mA); to be measured |

### 2.4 Tech Gateway measurement section
Rev A's core (S3-MINI-1-N4R2, LMR51420 buck, SDMMC, TCAN3414, USB-C path) plus:

| Measurement | Hardware | Notes |
|---|---|---|
| **Supply voltage** | Precision ADC or INA228-class monitor; ±10 mV | → `volt`; ≥ 500 Hz during a pulse test, 1 Hz otherwise |
| **Pulse load** | MOSFET + resistor, **100 mA only**; its current measured by its own shunt | Fail-safe off (gate pull-down) plus a hardware time-out; sized for 50 % duty for 10 s |
| **Termination** | §2.5 | → `term` |

**LEN 4 [must]:** ≈60 mA plus the 100 mA pulse. There is no 250 mA option, because a declared LEN must cover every load the device can draw. Lock-in detection at 7.3 Hz resolves microvolts, so 100 mA is enough.

### 2.5 Termination measurement (Tech Gateway) **[PoC]**
Goal: classify ≈60 Ω (both terminators), ≈120 Ω (one missing), ≈40 Ω (one extra) and open (none) on a live bus without disturbing traffic.
1. A photoMOS relay connects the test network to CAN-H/CAN-L only during a requested test.
2. A **±0.2 V differential source through ~1 kΩ**, either **floating relative to NET-C** or **biased to the bus common mode (≈2.5 V)**. A ground-referenced source drives common-mode current into every transceiver.
3. **Sample only during idle:** after ≥ 11 recessive bit times on the transceiver's RX output, trigger a fast **sample-and-hold ADC** (SAR, ≥ 500 kS/s, ≥ 14 bit, differential through an instrumentation amplifier). The trigger is a timer/capture peripheral on RX, not the TWAI driver.
4. Settling well under 40 µs; the PoC measures it.
5. Alternate polarity and take half the difference to cancel transceiver offsets; average a few hundred idle windows.
6. Report the raw value: R = R_series × V_meas / (V_src − V_meas). The app corrects for transceiver loading (≈10–100 kΩ per node) from the node count it sees, and classifies.

The PoC (§7 tests 1–3) gates the Tech Gateway layout. If it fails, the Tech Gateway ships without `term`; the app then shows error counters and "Check termination with a meter (network powered down)".

### 2.6 CAN error reporting (both, firmware)
TWAI alerts for bus errors, error-passive, bus-off and arbitration loss. Report TEC/REC, bus state, errors per minute, bus-off count and RX overruns. Report errors by type where ESP-IDF exposes them, otherwise `null`. Bus-off recovery is automatic and counted.

### 2.7 NMEA 2000 behaviour **[must]** (both)
* Address claim, ISO Request, Product Information (126996 with the measured LEN), Configuration Information (126998), Heartbeat, PGN list. The `NMEA2000` library (Timo Lappalainen) covers this.
* **Class 25 (Internetwork device), function 137 (NMEA 2000 Wireless Gateway).**
* Manufacturer code **[decide]** (none assigned; build-time setting).
* Transmits only its own network-management PGNs, plus app traffic under §2.8.

### 2.8 Security **[must]**
* Unique Wi-Fi access point password per unit, printed on the label.
* The API runs over **HTTPS with a per-unit self-signed certificate**. Native clients pin it on first use. Plain HTTP is refused except on the gateway's own access point.
* **Browsers never talk to a gateway.** The app's web and webview front-ends go through the Rust core.
* **At most 2 concurrent TLS clients** (≈40–60 KB of RAM each, in PSRAM), plus the RAW stream. The 3rd gets `503` with `Retry-After`.
* A bearer token is required for every command that changes something: tests, terminator, pairing, settings, transmit.
* **Transmit:** off by default. It is enabled per technician session, and transmit lines are accepted on the RAW port only from the client IP that enabled it. It turns off when that session ends.

### 2.9 Logging has priority **[must]** (both)
* Measurement features are off by default and run only during a **technician session** (`session.start` with the token; expires after 15 minutes without activity).
* No periodic tests outside a session.
* A measurement never pauses, slows or drops logging or the RAW stream: lower task priority, capped CPU time. Verified by §7 tests 2 and 12.
* The logging format (PGNs such as 128267, 129029/129025, 127257 and 127252; rates; file format; IHO crowdsourced bathymetry compatibility) is owned by the logger's spec.

### 2.10 Physical
| | Rev A | Tech Gateway |
|---|---|---|
| Bus connection | Pigtail, M16 gland | **Panel-mount male Micro-C** |
| USB | Native, not isolated, bench only | **Isolated at USB:** ADuM3160-class isolator (S3 native USB is full speed) plus a 1 W isolated 5 V → 5 V converter, so the board runs from USB with the bus unplugged |
| CAN, measurement and logic ground | NET-C | NET-C (bus-referenced; no CAN-side isolation) |

**Board ID:** one firmware image for both boards. A resistor-divider strap on a spare ADC pin, read at boot, selects the board and so its `caps`.

### 2.11 ESP-NOW coordinator (both)
Both boards pair with tees and forward their status (§6). Rev A shows the tee's own currents and voltage but can't run a pulse test.

---

## 3. The Power Tee
Rev 2 §3, accepted as written; summary:
* **Function:** replaces the power-insertion tee. Measures current per port and input voltage, and holds one switchable terminator. It reports over ESP-NOW only: no CAN transmit, no transceiver, no address. CAN-H, CAN-L and shield pass straight through.
* **Connectors:** female Micro-C port A, male Micro-C port B. Pins: 1 shield, 2 NET-S, 3 NET-C, 4 CAN-H, 5 CAN-L. Battery lead fused at the source (ABYC E-11). A removable shield-to-NET-C link, fitted by default.
* **Power path:** **4 A total**, copper sized for 6 A, current range to 6 A. One high-side shunt per port (≈10 mΩ) read by INA228-class monitors, which also measure input voltage. **Nothing active in the power path**; power flows if the electronics are dead. Input TVS; electronics fed through a PTC and a buck rated ≥ 40 V. Over-current warning above 4 A.
* **Accuracy:** current ±1 % ±5 mA; voltage ±0.5 % ±10 mV; ≥ 500 Hz per channel during a pulse test, 10 Hz otherwise.
* **Terminator:** one 120 Ω (1 %, ≥ 0.25 W) via a **latching** relay rated ≥ ±30 V. It holds through power loss, reboots and link loss, and its state is read back. It changes only on a gateway command (§5.3) or a 10 s button hold. Factory default off.
* **MCU:** **ESP32-C6 preferred** (newer radio, ESP-NOW long-range mode); C3 acceptable. **u.FL external-antenna variant [decide]**; the app side recommends offering it, for engine rooms and metal hulls.
* **Controls:** button: short press = link state; 3 s = pairing for 60 s; 10 s = toggle terminator. RGB LED: blue blinking = pairing; green = linked; slow amber = link lost; red = fault.
* **Temperature** near the shunts, warning above 70 °C. **Firmware** via USB-C in v1, over ESP-NOW in v2. **Label:** serial `PT-xxxx`, 6-digit code, QR code.

---

## 4. Gateway API (version 1) **[must]**

### 4.1 REST (HTTPS)
Units in field names (`_v`, `_a`, `_ohm`, `_c`, `_dbm`, `_ms`); `t_ms` = milliseconds since boot.

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /api/v1/info` | open | Identity and caps |
| `GET /api/v1/status` | open | Latest of everything |
| `GET /api/v1/events` | open to read; token to send commands | WebSocket: events and commands (§4.2) |
| `POST /api/v1/session` / `DELETE` | token | Start/extend or end a technician session |
| `POST /api/v1/termination/measure` | token + session | `term` only |
| `POST /api/v1/pulse` | token + session | `{ "freq_hz": 7.3, "seconds": 5 }` (100 mA fixed); `pulse` only |
| `GET /api/v1/tees` | open | Paired and nearby tees |
| `POST /api/v1/tees/pair` | token | `{ "serial": "PT-0057", "code": "482913" }` |
| `POST /api/v1/tees/{serial}/unpair` | token | |
| `POST /api/v1/tees/{serial}/terminator` | token + session | `{ "on": true, "confirmed_end": true }`; on rev A also `"meter_ohm": 118` (§5.3) |
| `POST /api/v1/tees/{serial}/identify` | token | Blink LED 10 s |
| `PUT /api/v1/settings` | token | `{ "raw_port": 1457, "wifi": {...} }` |
| `POST /api/v1/transmit` | token + session | `{ "on": true }`; binds to the caller's IP (§2.8) |

A capability the board doesn't have returns `404 {"error":"not_supported","cap":"pulse"}`.

### 4.2 WebSocket (one per app instance)
* **Events, server → app:** `{"type":"status",...}`, `termination`, `pulse_result`, `tee_status`, `tee_event`, `session`.
* **Commands, app → server:** `{"id": 42, "cmd": "pulse", "args": {"freq_hz": 7.3, "seconds": 5}, "token": "…"}`, with the same command names and arguments as the REST calls (`session.start`, `session.end`, `termination.measure`, `pulse`, `tee.pair`, `tee.unpair`, `tee.terminator`, `tee.identify`, `transmit`, `settings`).
* **Replies:** `{"id": 42, "ok": true, "result": {...}}` or `{"id": 42, "ok": false, "error": "..."}`.
* The token may be sent once with `auth` at connect instead of on each command.

### 4.3 Examples
Rev A `info`:
```json
{ "model": "gateway", "hw_rev": "A", "serial": "GW-7F3A", "fw": "0.4.0", "api": 1,
  "raw_port": 1457, "transmit": false, "session": false, "clients": 1, "max_clients": 2,
  "caps": ["err", "temp", "volt_coarse", "tee"],
  "n2k": { "address": 1, "name_hex": "C0A1B2C3D4E5F601", "len": 2, "class": 25, "function": 137 } }
```
Tech Gateway `info`: `"model": "tech-gateway"`, `"hw_rev": "A1"`, `"caps": ["err","temp","volt","term","pulse","tee"]`, `"len": 4`.

`status` (1 Hz). Rev A has `supply_v_coarse` and `"termination": null`; the Tech Gateway has `supply_v` and a `termination` object:
```json
{ "type": "status", "t_ms": 8040123, "uptime_s": 8040,
  "supply_v": 12.42, "temp_c": 41.2,
  "wifi": { "mode": "ap", "ssid": "Keelwire-7F3A", "channel": 6, "rssi_dbm": -58, "clients": 2, "reconnects": 0 },
  "can": { "state": "active", "tec": 0, "rec": 3, "bus_off_count": 0, "errors_per_min": 3.8,
           "by_type_10min": { "bit": 21, "stuff": 2, "crc": 0, "form": 11, "ack": 4 },
           "rx_frames": 1203344, "tx_frames": 211, "rx_overruns": 0 },
  "logging": { "active": true, "dropped": 0 },
  "termination": { "raw_ohm": 117.4, "t_ms": 8020001, "quality": "ok", "idle_samples": 412 } }
```
`termination.quality`: `ok`, `noisy`, `busy` (too few idle windows), `fault` (outside 20 Ω–10 kΩ), `off` (no session).

`pulse_result`:
```json
{ "type": "pulse_result", "id": 17, "amp_ma": 101.3, "freq_hz": 7.3, "seconds": 5,
  "gateway": { "dv_v": 0.0721, "v_mean": 12.42 },
  "tees": [ { "serial": "PT-0042", "dv_v": 0.0063, "di_a_a": 0.1009, "di_b_a": 0.0004, "v_mean": 12.71 } ] }
```

`tee_status` (1 Hz per paired tee):
```json
{ "type": "tee_status", "serial": "PT-0042", "fw": "0.1.0", "link": "ok", "link_rssi_dbm": -49, "age_ms": 180,
  "v_in": 12.71, "i_a_a": 0.742, "i_b_a": 0.000, "temp_c": 33.0,
  "terminator": { "on": false, "readback_ok": true }, "faults": [] }
```
When the link drops: `"link": "lost"`, `age_ms` keeps growing, and the values are the last known ones.

---

## 5. Methods

### 5.1 Termination (Tech Gateway)
On request, inside a technician session. Never during a pulse test.

### 5.2 Pulse test (Tech Gateway + tee)
The gateway switches 100 mA as a 7.3 Hz square wave for N seconds. Each unit demodulates at 7.3 Hz from its own samples; no clock sync is needed.
* The gateway reports ΔI of its load and ΔV of its supply.
* The tee reports ΔI per port and ΔV of its input.

The app derives:
* the side of the tee the gateway is on (the port whose ΔI ≈ the gateway's ΔI);
* the cable resistance R = (ΔV_tee − ΔV_gateway) / ΔI_gateway;
* the distance (from the selected cable type);
* the voltage at full declared load: V_tee − R × the declared current of that side.

Before pulsing, the app checks that the declared LEN total (or the tee's measured current) plus 100 mA stays within the network rating, and that the predicted minimum voltage stays above 9 V.

### 5.3 Turning the tee terminator on
All of these must hold.
1. **Termination says one missing.**
   - Tech Gateway: from its own fresh reading (< 60 s, quality `ok`).
   - Rev A: from a technician-entered `meter_ohm`, measured with the network powered down.
2. **The technician physically confirms** that nothing continues past one side of the tee (`confirmed_end: true`). Port current is supporting evidence only.
3. A **technician session** is active, and changes are unlocked in the app.
4. **Gateway interlock:** refuse when the governing raw value (`raw_ohm` or `meter_ohm`) is below 80 Ω.

Afterwards the Tech Gateway re-measures and returns both readings. On rev A, the app asks the technician to confirm that bus errors stop. If the result is wrong, the app offers to switch the terminator back.

### 5.4 Spot check (Tech Gateway)
Move the gateway to another drop and press **Measure here**. The app runs the termination and pulse tests and saves the result with a place name. It needs the panel Micro-C; it isn't offered for rev A.

---

## 6. ESP-NOW link
* The gateway broadcasts `HELLO` at 1 Hz on its Wi-Fi channel. Tees scan channels 1–13, follow the gateway's channel, and rescan after 5 s without a `HELLO`.
* **Range:** use the ESP-NOW long-range rate (C6), and the u.FL variant where needed. A lost link is normal: the terminator latches, the tee keeps measuring, and the gateway reports `link: lost` with last-known values. No wired fallback in v1 **[decide]**; a wired link over CAN would make the tee an NMEA 2000 device with an address claim and LEN.
* **Pairing:** hold the tee's button.
  - **X25519 ECDH** between tee and gateway; a passive listener can't derive the key.
  - The label's 6-digit code authenticates the exchange (a MAC over both public keys, keyed with the code; one online guess per attempt, rate-limited). SPAKE2 is acceptable instead.
  - The ESP-NOW LMK is derived via HKDF-SHA256. The QR code may carry a longer code.
  - Pairing is persisted on both sides.
* **Messages:** encrypted unicast with a sequence number. Packed little-endian; header `{ver u8, type u8, seq u16, serial[8]}`. Types: `HELLO`, `PAIR_KEX`, `PAIR_CONFIRM`, `STATUS` (1 Hz), `PULSE_START`, `PULSE_RESULT`, `TERM_SET`, `TERM_STATE`, `IDENTIFY`, `ACK`, and `OTA_*` in v2.
* **Delivery:** commands are retried until `ACK` (3 × 50 ms). Up to 4 tees per gateway.

---

## 7. Test plan

| # | Test | Board | Pass |
|---|---|---|---|
| 1 | **PoC:** bench bus with 2, 1, 0 and 3 terminators | Tech | Each case classified correctly |
| 2 | Test 1 at 30 % bus load with logging running | Tech | Same; no new CAN errors on other nodes; logging drops nothing |
| 3 | Test 1 with 50 nodes (real or simulated transceiver loads) | Tech | Classified correctly; raw value within the app's correction model |
| 4 | Remove one terminator on a 15 m backbone; tee terminator on with confirmation | Tech, then rev A with `meter_ohm` | ≈60 Ω afterwards (Tech); errors stop (both) |
| 5 | Pulse test, gateway on each side of the tee, 2 m and 20 m of cable | Tech | Correct side every time; R within ±10 % of a 4-wire measurement |
| 6 | Pull power from the tee with the terminator on | Tee | Terminator stays on |
| 7 | Hold the tee's MCU in reset | Tee | Network power unaffected |
| 8 | 4 A for 1 h at 40 °C; 6 A for 10 min | Tee | Below 70 °C; readings within spec |
| 9 | Tee in a closed engine compartment / metal box | Tee | Link quality logged; `link: lost` handled; terminator unaffected |
| 10 | Gateway rejoins a router on another channel | Both | Tee follows within 10 s |
| 11 | Unauthenticated or session-less command; plain HTTP on a shared network; a 3rd TLS client; RAW transmit from another IP | Both | 401 / refused / 503 / ignored |
| 12 | 24 h logging with technician tests every 5 min | Both | Logger drops nothing |
| 13 | USB to a shore-powered laptop while on the bus | Tech | No ground-loop current through NET-C; CAN unaffected. (Rev A: documented as bench-only USB) |
| 14 | `volt_coarse` against a meter, 9–16 V | Rev A | Within ±2 % |

## 8. Hand back to the app side
1. Termination PoC results (scope captures of the idle trigger; classification table).
2. API as built: `info` from both boards, a minute of WebSocket events saved to a file, and any field changes.
3. A 10-minute YD RAW capture from a real boat, with error counts for the same period.
4. Schematics and BOMs for all three units; the board-ID strap values; measured LEN for each.
5. §7 results as CSV; firmware versions and how to flash them (one image, board-ID strap).

The app side will build:
* an `keelwire:` connection (RAW stream + one WebSocket), with features from `caps`;
* a simulator for rev A, the Tech Gateway and the tee;
* the screens in the "Keelwire Network Map & Device Table" canvas, technician pack section.

## 9. Certification and liability (product-risk view, not legal advice)
* **FCC Part 15B / CE** for each finished product (the module's certification covers only the radio); **IEC 60945** for marine EMC.
* **NMEA 2000 certification** before using the name or logo, particularly for the gateways, which transmit.
* **Power Tee:** it carries the network's power, so it's the highest-liability item. Follow ABYC E-11 fusing guidance and keep the power path passive.

## 10. Later
Mini-C 8 A tee; separate end-cap terminator; inline drop probe; time-domain reflectometry; tee firmware over the air; a wired tee fallback.
