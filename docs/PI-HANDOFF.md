# Keelwire Hub: Linux module on or beside the gateway (exploration handoff)

**Status:** exploration, not committed. Rev A and the Tech Gateway stay as specified in `HARDWARE-HANDOFF.md`; nothing here changes them.
**Date:** 2026-10-27 · **From:** app side · **To:** hardware and firmware agent
**Related:** `HARDWARE-HANDOFF.md` (gateways, Power Tee, API v1), `SPEC.md` §1 item 2 (headless mode), §11.5 (service sessions)

---

## 0. Why this exists

The owner wants a technician to be able to walk up to a boat with **one box** and nothing to install. Three ways to get there:

| Path | What runs the Keelwire engine | Needs | Status |
|---|---|---|---|
| **1. Phone or tablet app** | The phone (same Rust engine, iOS/Android build) | Keelwire Gateway or any supported Wi-Fi adapter | Planned (secondary target). **App side recommends this as the primary path.** |
| **2. Companion Pi** | Raspberry Pi Zero 2W in its own small case, USB to the gateway | Rev A unchanged + one firmware feature (§3) | Cheap to try on the bench |
| **3. Keelwire Hub board** | Raspberry Pi CM0 soldered onto a gateway-derived board | New board | Only if path 1 falls short (see §6) |

Paths 2 and 3 serve the Keelwire web interface from the device itself, so **any** phone, tablet or laptop browser can use it with no app installed, and the device can keep recording and analysing when nobody is connected.

**What the gateway already covers:** rev A logs to microSD (§2.3, §2.9 of the main handoff). So a leave-behind recorder does **not** need Linux. The one gap is getting that log into a session (§3, which is wanted for path 1 as well).

---

## 1. Engine footprint (measured, app side)

| | Value |
|---|---|
| Memory | ≈36 MB resident (debug build, simulator with ~13 devices); release build expected lower |
| CPU | 1–2 % of one desktop core on simulated traffic; a full 250 kbit/s bus is ≈1 800 frames/s |
| Recording | ≈41 MB/hour pcapng on a moderately busy boat (≈1 GB/day) |
| Build | `aarch64-unknown-linux-gnu` is already produced by `release.yml` (`raspberry-pi-64bit`); runs on 64-bit Raspberry Pi OS Lite |

Not yet verified on Zero 2W / CM0 hardware: boot-to-ready time, RSS under a full bus, report generation time (PDF) on a 1 GHz A53.

---

## 2. Path 2: Companion Pi Zero 2W over USB

```
 phone / tablet / laptop browser
          │ Wi-Fi (Pi is the access point, or joins the gateway's)
 ┌────────┴────────┐  USB-C (power + data)  ┌─────────────────────┐
 │ Pi Zero 2W       │◄──────────────────────►│ Keelwire Gateway     │── drop ── NMEA 2000
 │ keelwire --headless│   YD RAW over CDC-ACM  │ (rev A or Tech)      │
 └─────────────────┘                         └─────────────────────┘
```

### 2.1 Data
* Gateway exposes **YD RAW over USB CDC-ACM** (same line format as §2.1 of the main handoff, same TX rules, default listen-only). **[ask]**
* App side adds `ydraw:serial:///dev/ttyACM0` to the driver list (TCP/UDP exist today; serial transport is new). **[app]**
* The gateway's own REST/WebSocket API (caps, temperature, terminator, Power Tee) stays on Wi-Fi/IP. Over USB, either (a) a second CDC interface carrying the API v1 WebSocket messages as JSON lines, or (b) USB-NCM networking so the existing HTTPS API just works. **[decide]** App side prefers (b): no second protocol.

### 2.2 Power
* Pi Zero 2W draws roughly 0.6 W idle and up to about 3 W under load (estimate; measure). At 12 V through an 85 % buck that is ≈60–300 mA.
* **If the gateway powers the Pi from the bus,** the gateway's declared LEN must cover it: ≈2 (gateway) + 6 (Pi peak) ≈ **LEN 8, to be measured**. Rev A's LMR51420 path and 5 V rail must be checked for ≥1 A at 5 V. Rev A's USB port is **not isolated**; powering a Pi from it is fine (the Pi floats), but a laptop plugged into the Pi then ties to NET-C, the same caveat as today.
* **Alternative:** Pi powered from its own USB power bank; gateway supplies data only. LEN unchanged. Simplest for a first bench test.

### 2.3 Storage and power loss
* Bus power can disappear without warning. Pi image: read-only root (overlayfs), data on a separate partition mounted `sync`/`commit=5`, pcapng written in rolling 5-minute chunks so a cut loses at most the open chunk.
* Optional hold-up: supercap or the gateway's existing 220 µF is not enough for a Pi (seconds needed, not milliseconds). Either accept chunk loss or add a ~5 F supercap with a power-fail GPIO to the Pi. **[decide]**

### 2.4 Wi-Fi
* Both radios are 2.4 GHz. Recommended: Pi runs the access point (`keelwire-XXXX`); gateway Wi-Fi off while the USB link is up (saves power and airtime), or gateway joins the Pi's AP on the same channel so ESP-NOW to the Power Tee keeps working. **[decide]** ESP-NOW must stay on the channel the gateway uses.

---

## 3. Gateway log fetch (wanted for every path) **[ask]**

When a phone app is backgrounded (iOS suspends it within seconds to minutes of locking the screen) or the Wi-Fi drops, the app misses traffic. The gateway is already recording to microSD, so let the app fill the gap:

* `GET /api/v1/log?from=<unix_ms>&to=<unix_ms>` → YD RAW lines (or the logger's native format plus a converter), streamed, gzip allowed.
* `GET /api/v1/log/index` → list of files with start/end times and sizes.
* Timestamps: gateway clock set by the app on connect (`POST /api/v1/time`) and from GNSS 126992/129029 when present.
* The app marks the session's pause, then backfills the capture from the gateway's log on reconnect, so the session stays continuous. Shown in the timeline as "Gap 09:12–09:14 filled from the gateway's card".

This makes the phone path (1) as complete as a Pi for attended work, and lets the gateway be the leave-behind recorder.

---

## 4. Path 3: Keelwire Hub board with a Raspberry Pi CM0

The CM0 is the Zero 2W's RP3A0 as a castellated, solder-down module (39 × 33 mm, 512 MB, 8/16 GB eMMC, optional Wi-Fi/BT, 1× USB 2.0, UART, SPI, I²C, −20 to +85 °C).

**Supply risk:** as of April 2026 it is officially sold only in China (through EDATEC); outside China it appears on AliExpress resellers from about $33 (8 GB, wireless), without official support or warranty. Raspberry Pi has said there are no plans to sell it elsewhere for now. A product depending on it needs a second source or a fallback (the Zero 2W as a daughterboard on pogo pins/headers).

If pursued:
* **Link to the ESP32-S3:** USB (S3 as device, CM0 as host) with the §2.1 CDC/NCM scheme, so firmware is shared with path 2. UART at 921 600 baud is too tight for a full bus in text form (≈70 KB/s); avoid.
* **Power:** same budget as §2.2, declared LEN ≈8 (measure); buck sized for 5 V 1.2 A; thermal check in the sealed enclosure (A53 at 1 GHz throttles at 80 °C).
* **Radios:** CM0 Wi-Fi as AP for users; S3 Wi-Fi reserved for ESP-NOW to the Power Tee, on the same channel. One u.FL each, antennas ≥20 mm apart or a shared-antenna design.
* **Storage:** eMMC for OS (read-only root), microSD stays on the S3 for the raw log (logging keeps priority and survives a Linux crash).
* **Software:** a Keelwire OS image (Raspberry Pi OS Lite or Buildroot), `keelwire --headless` as a systemd service, updates by signed image or `.deb` from GitHub releases. The app side would own the image build.
* **Certification:** a new radio and a new module mean the Hub needs its own RF and EMC testing; the CM0's module approvals help but do not cover the finished product.

---

## 5. What the app side does regardless

1. Mobile build of the engine (Tauri 2, iOS/Android), connecting to Keelwire gateways and existing adapters (YD RAW TCP/UDP, Actisense, Signal K).
2. Serial transport for YD RAW (`ydraw:serial://`), for path 2/3 and for USB-connected gateways on laptops.
3. Session backfill from the gateway log (§3) once the endpoint exists.
4. Headless session download page: on a Pi, sessions and reports are listed in the web interface and download to the viewing device.

## 6. Recommendation and decision points

* **Recommended order:** phone/tablet app first (path 1) with gateway log fetch (§3); companion Pi Zero 2W as a bench experiment (path 2), which needs only §2.1 on the gateway; Hub board (path 3) only if field use shows that installing the app is the real obstacle, and only once CM0 supply outside China is settled.
* **Owner decisions:**
  1. Add §3 (log fetch) to the gateway firmware v1? App side: yes.
  2. Add YD RAW over USB CDC (§2.1) to firmware v1? App side: yes, small and useful for laptops too.
  3. USB-NCM for the API over USB, or a second CDC channel?
  4. Pursue the Hub board now, or revisit after field tests of the phone app?

## 7. Hand back
* Measured Zero 2W current (idle, full bus, report generation) from the gateway's 5 V rail; whether rev A's rail can carry it.
* Firmware: RAW over CDC-ACM; log fetch endpoints and file format.
* If path 3 goes ahead: CM0 sourcing plan, block diagram, power and thermal budget, antenna layout.
