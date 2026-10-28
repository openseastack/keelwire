#!/bin/bash
# Build and run Keelwire on iPhone and iPad from this Mac (docs/IOS-SPEC.md §9).
#
#   bash scripts/build-ios.sh sim          run in the iOS Simulator (pick an iPhone or iPad)
#   bash scripts/build-ios.sh device       run on your iPhone/iPad connected by USB
#   bash scripts/build-ios.sh xcode        open the Xcode project
#   bash scripts/build-ios.sh testflight   build for the App Store and upload to TestFlight
#   bash scripts/build-ios.sh reset        delete the generated Xcode project (re-created next run)
#
# Needs: full Xcode (from the App Store, opened once), Homebrew, Node.js.
# Signing: sign in to Xcode › Settings › Accounts with your Apple developer
# account, and set your Team ID (developer.apple.com › Account › Membership):
#   export APPLE_DEVELOPMENT_TEAM=ABCDE12345
# TestFlight also needs an App Store Connect API key:
#   export APPLE_API_KEY_ID=...  APPLE_API_ISSUER=...
#   and the key file at ~/.appstoreconnect/private_keys/AuthKey_<APPLE_API_KEY_ID>.p8
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
CMD=${1:-sim}
# Everything is also written to build-ios.log (handy when asking for help).
exec > >(tee "$ROOT/build-ios.log") 2>&1
TAURI_DIR=crates/keelwire-tauri

say() { printf '\n== %s ==\n' "$*"; }
die() { printf '\n%s\n' "$*" >&2; exit 1; }

if [ "$CMD" = "reset" ]; then
  rm -rf "$TAURI_DIR/gen/apple"
  echo "Removed $TAURI_DIR/gen/apple; it is generated again on the next build."
  exit 0
fi

say "Keelwire for iPhone and iPad: checking this Mac"

DEV_DIR=$(xcode-select -p 2>/dev/null || true)
case "$DEV_DIR" in
  *Xcode*.app*) ;;
  *) die "Full Xcode is needed (not only the command line tools).
Install Xcode from the App Store, open it once, then run:
  sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
  sudo xcodebuild -license accept" ;;
esac

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v rustup >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  if command -v cargo >/dev/null 2>&1; then
    echo "Rust is installed at $(command -v cargo) without rustup (probably Homebrew)."
    echo "iPhone builds need rustup to add the iOS targets; installing it alongside (one time, about a minute)..."
  else
    echo "Installing Rust (one time, about a minute)..."
  fi
  # Allow rustup next to an existing (e.g. Homebrew) Rust; this script puts rustup's first on PATH.
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | RUSTUP_INIT_SKIP_PATH_CHECK=yes sh -s -- -y --profile minimal --no-modify-path
fi
# Use rustup's Rust for this build (it has the iOS targets), ahead of any other Rust.
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
echo "Rust: $(cargo --version) at $(command -v cargo)"

if ! command -v node >/dev/null 2>&1 && [ -s "$HOME/.nvm/nvm.sh" ]; then
  . "$HOME/.nvm/nvm.sh"
fi
command -v node >/dev/null 2>&1 || die "Node.js is needed. Install the LTS version from https://nodejs.org and run this again."

# XcodeGen generates the Xcode project; CocoaPods links Tauri's iOS libraries.
# Installed with Homebrew (Tauri would otherwise try `sudo gem install cocoapods`).
for tool in xcodegen:xcodegen pod:cocoapods; do
  cmd=${tool%%:*}; formula=${tool##*:}
  if ! command -v "$cmd" >/dev/null 2>&1; then
    command -v brew >/dev/null 2>&1 || die "Homebrew is needed to install $formula: see https://brew.sh"
    echo "Installing $formula..."
    brew install "$formula"
  fi
done

if [ -z "${APPLE_DEVELOPMENT_TEAM:-}" ]; then
  echo "Note: APPLE_DEVELOPMENT_TEAM is not set. Tauri uses the team of the signing"
  echo "certificate in your keychain if there is exactly one; otherwise set it:"
  echo "  export APPLE_DEVELOPMENT_TEAM=<your 10-character Team ID>"
fi

# The Tauri CLI is pinned in crates/keelwire-tauri/package.json. Running it with
# `npm run tauri` from there matters: the generated Xcode project calls back
# into it the same way to compile the Rust code.
TAURI="npm run --silent tauri --"

say "Installing interface packages"
(cd ui && npm ci --no-audit --no-fund)
(cd "$TAURI_DIR" && npm install --no-audit --no-fund)

# A setup that stopped half-way (e.g. a missing tool), or one made by an older
# version of this script, is redone from scratch.
INIT_OK="$TAURI_DIR/gen/apple/.keelwire-init-v2"
if [ ! -f "$INIT_OK" ]; then
  say "Generating the Xcode project"
  rm -rf "$TAURI_DIR/gen/apple"
  (cd "$TAURI_DIR" && $TAURI ios init)
  touch "$INIT_OK"
fi

case "$CMD" in
  sim)
    say "Building and starting in the Simulator (first build takes 5–10 minutes)"
    echo "Pick an iPhone or an iPad from the list."
    (cd "$TAURI_DIR" && $TAURI ios dev)
    ;;
  device)
    say "Building for your iPhone/iPad"
    echo "Connected devices:"
    xcrun devicectl list devices 2>/dev/null || true
    NAME=${2:-}
    if [ -z "$NAME" ]; then
      echo
      echo "If your iPhone/iPad is not listed as available: connect it with a cable,"
      echo "unlock it, tap Trust, and turn on Settings › Privacy & Security › Developer Mode."
      echo "Then run:  bash scripts/build-ios.sh device \"<name from the list>\""
      echo "(Without a name, Tauri asks, or picks a simulator if no device is ready.)"
      (cd "$TAURI_DIR" && $TAURI ios dev)
    else
      (cd "$TAURI_DIR" && $TAURI ios dev "$NAME")
    fi
    ;;
  xcode)
    (cd "$TAURI_DIR" && $TAURI ios open)
    echo "Tip: build from this script rather than Xcode's Run button; Xcode does not see Cargo and Node on its own."
    ;;
  testflight)
    : "${APPLE_API_KEY_ID:?set APPLE_API_KEY_ID (App Store Connect › Users and Access › Integrations)}"
    : "${APPLE_API_ISSUER:?set APPLE_API_ISSUER (shown above the keys table)}"
    KEY="$HOME/.appstoreconnect/private_keys/AuthKey_${APPLE_API_KEY_ID}.p8"
    [ -f "$KEY" ] || die "Put the downloaded key at $KEY"
    # Automatic signing with the API key (Tauri reads these).
    export APPLE_API_KEY="$APPLE_API_KEY_ID" APPLE_API_KEY_PATH="$KEY"
    BUILD=${BUILD_NUMBER:-$(date +%Y%m%d%H%M)}
    say "Building the App Store version (build $BUILD)"
    (cd "$TAURI_DIR" && $TAURI ios build --export-method app-store-connect --config "{\"bundle\":{\"iOS\":{\"bundleVersion\":\"$BUILD\"}}}")
    IPA=$(ls -t "$TAURI_DIR"/gen/apple/build/arm64/*.ipa | head -1)
    say "Uploading $IPA to App Store Connect"
    xcrun altool --upload-app --type ios --file "$IPA" --apiKey "$APPLE_API_KEY_ID" --apiIssuer "$APPLE_API_ISSUER"
    echo "Uploaded. It appears in App Store Connect › TestFlight after Apple processes it (usually 10–30 minutes)."
    ;;
  *)
    die "Unknown command '$CMD'. Use: sim, device, xcode, testflight or reset."
    ;;
esac
cd "$ROOT"
