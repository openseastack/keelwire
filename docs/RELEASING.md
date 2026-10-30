# Releasing

Installers for macOS, Windows and Linux are built by GitHub Actions
(`.github/workflows/release.yml`). macOS and Windows packages have to be built on those
operating systems, which is what the hosted runners provide; nobody needs a Mac or a
Windows machine to make a release.

## Make a release

1. Set the same version in `Cargo.toml` (`[workspace.package] version`) and
   `crates/keelwire-tauri/tauri.conf.json` (`version`). The workflow refuses a mismatch.
2. Commit, then tag and push:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. Wait for the **Release** workflow (about 20–30 minutes). It creates a **draft** release with:
   - `Keelwire_<v>_universal.dmg` (macOS, Apple Silicon + Intel)
   - `Keelwire_<v>_x64-setup.exe` and `_x64_en-US.msi` (Windows)
   - `Keelwire_<v>_amd64.AppImage`, `.deb`, `.rpm` (Linux)
   - portable single-program archives for Linux, macOS, Windows and Raspberry Pi (64- and 32-bit)
4. Edit the draft's notes (template in `docs/release-notes-template.md`) and publish it.

The workflow can also be started by hand: Actions → Release → Run workflow → enter the tag.

## Code signing (recommended before handing out widely)

Unsigned packages work, but macOS and Windows show a first-launch warning (see `INSTALL.md`).
Add these repository secrets (Settings → Secrets and variables → Actions) and the next release
is signed automatically.

**macOS** (needs an Apple Developer account, 99 USD a year):

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application certificate exported as .p12, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | Password of that .p12 |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | Apple ID, an app-specific password, and team ID, for notarization |

**Windows** (a code-signing certificate from a CA):

| Secret | What |
|---|---|
| `WINDOWS_CERTIFICATE` | The .pfx, base64-encoded |
| `WINDOWS_CERTIFICATE_PASSWORD` | Its password |

Certificates kept in cloud HSMs (Azure Trusted Signing, SignPath for open-source projects)
need a different signing step; the Tauri documentation covers both.

## Building locally

On a Mac, `bash scripts/build-mac.sh` installs anything missing (Apple command line tools, Rust), builds `Keelwire.app` and a `.dmg` for this Mac, and opens the app.

On any OS, by hand:

```bash
cargo install tauri-cli --version "^2" --locked
cd ui && npm ci && cd ..
cd crates/keelwire-tauri && cargo tauri build       # installers for this computer's OS
```

Linux needs `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`. The results
are under `target/release/bundle/`.

## Mobile (next)

The same interface can be packaged for iOS and Android with Tauri's mobile targets
(`cargo tauri ios init` / `android init`), connecting to Wi-Fi gateways. This needs Xcode (on a
Mac) and the Android SDK/NDK, plus App Store and Play Store developer accounts. Not set up yet.

## Statistics

**Downloads and stars** are saved every day by the **Stats** workflow (`.github/workflows/stats.yml`) as CSV
files on the `stats` branch: `downloads.csv` (cumulative count per release file per day), `repo.csv`
(stars, forks, watchers), and a `README.md` with the latest totals. Nothing to set up; run it once by hand
from Actions → Stats → Run workflow to create the branch.

**Views, clones and referrers** (GitHub keeps only 14 days) need one extra secret, because the token
GitHub Actions provides can't read traffic:

1. Create a token: github.com → Settings → Developer settings → Fine-grained tokens → Generate.
   Repository access: only `keelwire`. Permissions: **Administration: Read-only** (that's where GitHub
   puts traffic). Set an expiry and a reminder to renew it.
2. Add it to the repository: Settings → Secrets and variables → Actions → New secret, name `STATS_TOKEN`.

The next run adds `views.csv`, `clones.csv`, `referrers.csv` and `paths.csv`.

**Product tour visits** are counted with [GoatCounter](https://www.goatcounter.com) (free for open-source
and non-commercial sites; no cookies or personal data). Create a GoatCounter site with the code `keelwire`;
if that code is taken, change the address in the last `<script>` of `site/index.html`. Counting only runs
on the published site (openseastack.org or `*.github.io`), never on local copies.


## iPhone and iPad (TestFlight)

See `docs/IOS-SPEC.md` §9. Locally: `bash scripts/build-ios.sh sim` (Simulator), `device` (your iPhone/iPad),
`testflight` (upload). In CI, `.github/workflows/ios.yml` runs on every version tag: with the secrets
`APPLE_API_ISSUER`, `APPLE_API_KEY_ID`, `APPLE_API_KEY_P8` and `APPLE_DEVELOPMENT_TEAM` it uploads a signed
build to TestFlight; without them it only checks that the Simulator build compiles.
