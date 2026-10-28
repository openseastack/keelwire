# Keelwire for iPhone and iPad — Specification

**Revision:** 1.0 (2026-10-27) · **Status:** phase 1 in progress · **Parent:** `SPEC.md` (rev 2.8)
**Owner decisions so far:** Apple Developer account (individual for now, moving to the LLC/nonprofit later); TestFlight for testers; phones and tablets are a secondary target after desktop.

---

## 1. Goal

A technician walks onto a boat with only a phone or tablet and a Wi-Fi NMEA 2000 gateway (a Keelwire Gateway or an existing one) and does a full service session: see the network, fix instancing and settings, record the bus, and hand the owner a report. Nothing to install on the boat, no laptop.

## 2. One app, iPhone and iPad

* **One universal app** (`org.keelwire.app`), the same features on both. The generated Xcode project targets iPhone and iPad (XcodeGen default device family 1,2) and supports all iPad orientations, so it works in Split View and Slide Over.
* **Layout follows the window width, not the device:**

| Width | Who | Layout |
|---|---|---|
| < 640 pt | iPhone; iPad in Slide Over or a narrow split | Phone layout: bottom tab bar; map as a vertical list; tables collapse to cards |
| ≥ 640 pt | iPad full screen, larger splits, iPhone Plus/Max in landscape | Desktop layout: left rail, map, tables, side panels |

* iPad with a keyboard gets the desktop keyboard shortcuts (1–6 for sections, `/` for search).
* Minimum: **iOS / iPadOS 16**. Wake Lock and modern WebKit features used by the UI are reliable from 16; about 95 % of active devices run 16 or later.
* Touch: every control is at least 44 × 44 pt; hover-only information has a tap equivalent; the map's "Arrange backbone" drag works with a long press (WebKit drag and drop) and has Move up / Move down buttons as a fallback (phase 2).

## 3. Architecture

Identical to desktop (SPEC §3): the Rust engine and the HTTP/WebSocket API run **inside the app** on `127.0.0.1` (random port) and a WKWebView shows the same React interface. No server, no cloud.

```
 ┌─────────────── Keelwire.app (iOS) ───────────────┐
 │  WKWebView  ── http://127.0.0.1:<port> ──►  Axum API + engine (Rust, tokio)  │
 │      ▲                                               │                       │
 │      └── platform hooks (share sheet, keep awake) ◄──┘                       │
 └──────────────────────────────────────────────────────────────────────────────┘
                     │ Wi-Fi (TCP)
          Keelwire Gateway · Yacht Devices · Actisense W2K-1 · Maretron IPG100 · Signal K
```

* Tauri 2 mobile: `keelwire-tauri` is a library with `#[tauri::mobile_entry_point]` (`crate-type = staticlib, cdylib, rlib`); `main.rs` calls it on desktop.
* **Platform hooks:** the engine exposes a small `Platform` trait the app implements (share a file, keep the screen awake). The web UI reaches them through the local API (`POST /api/v1/native/share`), so the UI needs no Tauri IPC and stays identical in a browser.
* Native code is Rust using `objc2-ui-kit` (already a Tauri dependency on iOS); no Swift plugin is needed for phase 1.

## 4. Gateways on iOS

| Connection | iOS | Notes |
|---|---|---|
| Keelwire Gateway (YD RAW over TCP) | ✔ | Primary |
| Yacht Devices YDWG-02 / YDEN-02, TCP RAW | ✔ | |
| Actisense W2K-1 (N2K ASCII over TCP) | ✔ | |
| Maretron IPG100 | ✔ | |
| Signal K (keelwire-tap plugin) | ✔ | |
| Simulator ("Demo boat") | ✔ | Also used by App Review |
| Replay a capture / open a boat file | ✔ | From the Files app (phase 2: document picker) |
| YD RAW over **UDP broadcast** | ✗ phase 1 | Receiving broadcast/multicast needs Apple's *Multicast Networking* entitlement (request form, approval by Apple). Requested in phase 2 |
| Automatic gateway discovery (mDNS `_keelwire._tcp`) | ✗ phase 1 | Phase 2 via Bonjour (`NWBrowser`), which needs no multicast entitlement; meanwhile the app remembers recent gateways and offers the usual default addresses |
| USB / serial (NGT-1, USB100), SocketCAN | ✗ | Not available on iPhone/iPad; hidden in the interface list |

## 5. Permissions and Info.plist (`crates/keelwire-tauri/Info.ios.plist`, merged at build)

| Key | Value / reason |
|---|---|
| `NSLocalNetworkUsageDescription` | "Keelwire connects to your NMEA 2000 Wi-Fi gateway on the boat's network." iOS asks once, on the first connection. If refused, the Connect screen explains how to allow it in Settings → Privacy → Local Network |
| `NSAppTransportSecurity › NSAllowsLocalNetworking` | lets the web view load the in-app server on `127.0.0.1` over HTTP |
| `UIFileSharingEnabled`, `LSSupportsOpeningDocumentsInPlace` | sessions and reports appear in **Files › On My iPhone/iPad › Keelwire** |
| `ITSAppUsesNonExemptEncryption` = false | no custom encryption; skips the export question on every upload |
| `UIRequiresFullScreen` = false | iPad multitasking |
| Document type `.keelwire` | open boat files from Files / AirDrop (from the existing file association) |

No location, camera, Bluetooth or tracking. Privacy label: **Data Not Collected**. A `PrivacyInfo.xcprivacy` manifest declares the required-reason APIs used (file timestamps; `UserDefaults` through WebKit storage) — phase 1 checklist item before the first external TestFlight build.

## 6. Files, reports and sharing

* Data (settings, open session) in `Library/Application Support/keelwire`; history in `Documents/Keelwire/Vessels/<Boat (MMSI)>/<date time>/` — visible in the Files app, backed up with the device.
* Where desktop shows **Download**, iOS shows **Share**: the system share sheet (AirDrop, Mail, Messages, Save to Files, print). On iPad the sheet anchors to the button's area. Typical use: *End session → Share owner report → Mail* to the owner.
* The HTML technician report also opens inside the app; PDFs open from the share sheet or Files (Quick Look preview in phase 2).

## 7. App lifecycle (the one big difference from desktop)

iOS suspends an app a few seconds after it leaves the screen or the screen locks; a suspended app receives nothing from the gateway.

| Event | What Keelwire does |
|---|---|
| Session recording, app in front | Screen kept awake (idle timer off) so the recording is not cut by auto-lock. Restored when no session is recording |
| App goes to the background / screen locked | Session paused with reason *background* ("Paused: app in the background"); the pause is logged and reported like a disconnect |
| App comes back | Local server checked (re-bound on the same port and the page reloaded if iOS reclaimed it); gateway reconnects (drivers retry with back-off); the session resumes on its own if paused for *background* or *disconnected* less than 30 min ago |
| App terminated while away | Next launch reopens the open session and asks "continue or start new" (as desktop) |
| Gap filling | Phase 3: when the gateway can serve its microSD log by time range (`PI-HANDOFF.md` §3), the app back-fills the gap so the capture is continuous |

A background-audio or VoIP trick to stay alive is **not** used: App Review rejects it.

## 8. App Review

* Reviewers have no boat: the Connect screen offers **Demo boat** (the built-in simulator, with the AIS transponder "SECOND WIND" and an instance conflict to fix). Review notes explain it.
* Category: Utilities (or Navigation). Age rating 4+. Free.
* Screenshots: 6.9" iPhone and 13" iPad (Overview, Map, Session, Owner report), generated from the simulator like the website's.

## 9. Building and testing

**Your Mac (one-time):** full Xcode (opened once), `brew install xcodegen cocoapods`, `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`, Node 22. Then `scripts/build-ios.sh`:

| Command | Does |
|---|---|
| `scripts/build-ios.sh sim` | Runs in the iOS Simulator (any iPhone or iPad model) |
| `scripts/build-ios.sh device` | Runs on your iPhone/iPad over USB (Developer Mode on; signs with your team) |
| `scripts/build-ios.sh xcode` | Opens the project in Xcode |
| `scripts/build-ios.sh testflight` | Builds the App Store IPA and uploads it to TestFlight |

* Team ID: `APPLE_DEVELOPMENT_TEAM` (10 characters, from developer.apple.com → Membership) or `bundle > iOS > developmentTeam` in `tauri.conf.json`. Not committed while the account is personal.
* The Xcode project (`crates/keelwire-tauri/gen/apple`) is generated by `tauri ios init` and not committed; everything that matters lives in `tauri.conf.json`, `Info.ios.plist` and the icons.

**TestFlight from GitHub Actions** (`release.yml`, job `ios`, macOS runner): runs when these repository secrets exist, otherwise skipped:

| Secret | From App Store Connect → Users and Access → Integrations → App Store Connect API |
|---|---|
| `APPLE_API_ISSUER` | Issuer ID |
| `APPLE_API_KEY_ID` | Key ID (Admin or App Manager role) |
| `APPLE_API_KEY_P8` | Contents of `AuthKey_<id>.p8` (downloadable once) |
| `APPLE_DEVELOPMENT_TEAM` | Team ID |

Builds are signed automatically with the API key. Version = workspace version; build number = GitHub run number.

**First-time App Store Connect setup:** create the app (My Apps → +) with bundle ID `org.keelwire.app`, name "Keelwire" (or "Keelwire NMEA 2000" if taken), SKU `keelwire-ios`. Internal testers (up to 100, no review) under TestFlight → Internal Testing.

## 10. Phases

| Phase | Scope | Status |
|---|---|---|
| **1** | App shell (mobile entry, iOS config, Info.plist, data folders); TCP gateways and Demo boat; sessions with background pause/resume; keep-awake; share sheet for reports and files; safe areas and iPhone/iPad layouts; build script; TestFlight CI | In progress (code written, needs first build on a Mac) |
| 2 | Bonjour discovery of Keelwire gateways; multicast entitlement for YD UDP; document picker for boat files and captures; Quick Look preview; Move up/down in Arrange; PrivacyInfo manifest; App Store screenshots | Planned |
| 3 | Gap back-fill from the gateway's log; Live Activity / lock-screen session timer; Android (same code, Play Store) | Planned |

## 11. Risks

| Risk | Mitigation |
|---|---|
| iOS reclaims the local server's socket while suspended | Health check on resume; re-bind on the same port; reload the page |
| Local Network permission refused | Clear message and link to Settings; Demo boat still works |
| WebKit drag and drop on iPhone | Long-press drag; Move up/down fallback (phase 2) |
| App Review: "needs hardware" | Demo boat and review notes |
| Code built on Linux CI cannot be tested for iOS | iOS type-check (`cargo check --target aarch64-apple-ios`) in CI; device testing on the owner's iPhone/iPad and TestFlight |
