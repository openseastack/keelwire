# Build status (2026-10-26)

Verified in a Linux build environment (Rust 1.97, Node 22):

| Part | State |
|---|---|
| `cargo test` (workspace) | 68 tests pass: 56 core (incl. CANboat-database and community Maretron decoding), 2 capture tests (a real 50k-frame CANboat SocketCAN capture and the Python sender's output), 9 driver/simulator tests, 1 Maretron-action test |
| `cargo clippy --tests` | No warnings |
| `keelwire` server | Runs against `sim:`. Exercised end to end: discovery (product info, descriptions, PGN lists for all 10 simulated devices), decode, filter, instance-conflict detection, arming guard, device-instance and installation-description changes with ack + read-back, data-instance command, device NAK reporting, switch control, pcapng capture, boat-file export, live pcap stream, Wireshark extcap (valid pcap, linktype 227), `dump` replay of a real capture |
| Web UI | Type-checks and builds; all five pages screenshot-checked in Chromium, no console errors |
| Desktop app (`keelwire-tauri`) | Builds and runs on Linux (WebKitGTK) — window opens on the embedded server. macOS/Windows bundles to be built via CI or `cargo tauri build` |
| Maretron compass actions, tank capacity, Maretron-style switching | Exercised end to end against the simulator; tank-capacity bytes asserted identical to the hardware-confirmed signalk-tlm100-config encoding |
| Single binary | `cargo build` builds the web UI via build.rs and embeds it; verified by deleting ui/dist and rebuilding. Port: `--port`, default 8080 falls through to the next free port (tested with 8080 occupied); browser opens automatically in desktop use |
| Themes | Day, Dark (new; follows OS preference on first start), Dusk, Night (red) — screenshot-checked |
| Feature checklist (spec Appendix C) | Device list columns/sort/chooser/print/CSV, Transmitted/Received PGN windows, units, 30 s instancing analysis with highlight, Data fields tab — screenshot-checked; a simulated tank-instance conflict was found and fixed entirely through the UI (analysis then reports none) |
| Standard field commands | Depth offset, temperature source, temperature instance, fluid type confirmed by read-back against the simulator; encoder byte-identical to the hardware-confirmed tank-capacity command (unit test) |
| Interface redesign (spec §13 rev 2.4) | Overview, Devices (by system / table / map, device panel with staged settings), Traffic (latest per message, every frame, detail with byte map), Switching, Connect and Settings screenshot-checked in Day, Dark and Night and at phone width, no console errors. Exercised against the simulator: one-click instance fix (confirmed), staged apply of description + depth offset (confirmed), switching after unlock-in-place |
| Not tested on hardware | SocketCAN, NGT-1, IPG100, YD gateways (protocols implemented per documentation; no device available in the sandbox) |
| Pending | CLMD12/16, CKM12 and TLM calibration-table layouts; USB100 protocol (spec Appendix A.5) |

## Packaging (2026-10-26)

- `.github/workflows/release.yml`: pushing a `v*` tag builds a draft GitHub Release with the macOS universal DMG,
  Windows setup.exe + MSI, Linux AppImage/.deb/.rpm, and portable single-program archives (Linux, macOS,
  Windows, Raspberry Pi 64/32-bit). Optional Apple/Windows signing via repository secrets (docs/RELEASING.md).
- Built and launch-tested here: Linux AppImage (80 MB, self-contained), .deb and .rpm (5.4 MB; depend on the
  system WebKitGTK), portable linux-x86_64 tar.gz (3.2 MB). AppImage shows the full Overview with the simulator.
- Not yet run: the macOS and Windows jobs (need GitHub's hosted runners; no repo pushed yet).
- Desktop app allows changes by default (in-app unlock still required); `--read-only` disables them.
- Mobile (iOS/Android via Tauri mobile): not started.

## Keelwire rename, map and tables (2026-10-27)

- Project named Keelwire everywhere; new icon set (desktop, iOS, Android, favicon).
- Devices → Map: backbone map (alternating / one side / phone), card fields (max 3), order-unknown grouping,
  Arrange backbone (drag, terminators, power tee; saved in browser storage), bus-health strip.
- Devices → Table: traffic columns, grouped column chooser, filter chips, search by name/@address/serial/PGN,
  expandable per-device PGN breakdown, By PGN view, CSV. Server: `GET /api/v1/stats`, `POST /api/v1/stats/reset`.
- Simulator fix: 127506 is now sent as fast-packet (was counted as incomplete).
- Product tour: `site/index.html` (GitHub Pages workflow `.github/workflows/pages.yml`).
- Tests: 69 pass; clippy clean.

## Service sessions and reports (2026-10-27, SPEC §11.5)

- A session starts when a gateway connects; the REC button (top right) shows recording / paused / reconnecting;
  a disconnect pauses and the session resumes on reconnect; after 30 min away or a restart, Keelwire asks
  whether to continue or start a new one.
- The boat is identified from its own AIS transponder (simulator: SECOND WIND, MMSI 367550123); confirm dialog;
  name and HIN can be typed when there is no AIS. Technician name is remembered.
- Change log of every device-changing request (before/after, method, device read-back), notes, findings
  tracked as fixed/open, load statistics, GPS position.
- End session: owner report (PDF), technician report (PDF or HTML), bundle (.zip) with pcapng, boat files
  before/after, changes/devices/pgns CSV; optional removal of other ships' AIS from the capture and boat files.
  Filed under Documents/Keelwire/Vessels/<Boat (MMSI)>/<date time>/ (or <data dir>/Vessels when headless).
- Boats page (history, since-last-visit, report links) and a Welcome-back banner on the Overview.
- PDFs come from a small built-in writer (`crates/keelwire-server/src/pdf.rs`) with embedded Geist fonts:
  no external PDF tool, works on a Raspberry Pi.
- Tests: 74 pass; clippy clean.

## iPhone and iPad, phase 1 (2026-10-27, docs/IOS-SPEC.md)

- One universal app (`org.keelwire.app`, iOS/iPadOS 16+): the same engine and interface run inside the app;
  phone layout under 640 pt, desktop layout on iPad.
- `keelwire-tauri` is now a library with a mobile entry point (`src/lib.rs`); `Info.ios.plist` adds the
  Local Network prompt, localhost loading, Files-app access; `bundle.iOS.minimumSystemVersion` 16.0.
- iOS lifecycle: session pauses when the app leaves the screen and resumes when it returns; the in-app
  server is checked and restarted if iOS closed it; screen kept awake while a session records.
- New crate `keelwire-ios` (objc2, no Swift): share sheet and keep-awake. Server: `Platform` hook,
  `POST /api/v1/native/share`, `POST /api/v1/native/save`; status reports `platform` and `can_share`.
- Interface: Share instead of Download on iPhone/iPad (reports, CSV, boat files, captures, device settings);
  report viewer; Wi-Fi gateway form, recent gateways and Demo boat on Connect; safe areas.
- Build: `scripts/build-ios.sh sim|device|xcode|testflight`; CI `ios-check` (type-check for iOS) and
  `.github/workflows/ios.yml` (simulator build, or TestFlight upload with App Store Connect secrets).
- Verified here: engine, server and `keelwire-ios` type-check for aarch64-apple-ios; the iOS-only app code
  type-checks on desktop; screenshots at iPhone and iPad sizes. Not yet run on a real iPhone/iPad or in
  the iOS Simulator (needs a Mac with Xcode).
