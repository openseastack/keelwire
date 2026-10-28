#!/bin/bash
# Build the Keelwire Mac app on this Mac and open it.
#   bash scripts/build-mac.sh
# Installs what's missing (Apple command line tools, Rust) on first run.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== Keelwire: building the Mac app =="

if ! xcode-select -p >/dev/null 2>&1; then
  echo "Apple's command line tools are needed. A window will open to install them."
  echo "When it finishes, run this script again."
  xcode-select --install || true
  exit 1
fi

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v cargo >/dev/null 2>&1; then
  echo "Installing Rust (one time, about a minute)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi

if ! command -v node >/dev/null 2>&1 && [ -s "$HOME/.nvm/nvm.sh" ]; then
  . "$HOME/.nvm/nvm.sh"
fi
if ! command -v node >/dev/null 2>&1; then
  echo "Node.js is needed to build the interface. Install the LTS version from https://nodejs.org and run this again."
  exit 1
fi

echo "Installing interface packages..."
(cd ui && npm ci --no-audit --no-fund)

echo "Building. The first build takes 5 to 10 minutes; later ones are much faster."
if ! (cd crates/keelwire-tauri && npx --yes @tauri-apps/cli@2 build --bundles app,dmg); then
  if ! command -v cargo-tauri >/dev/null 2>&1; then
    echo "Installing the Tauri build tool with Cargo instead (one time, about 5 minutes)..."
    cargo install tauri-cli --version "^2" --locked
  fi
  (cd crates/keelwire-tauri && cargo tauri build --bundles app,dmg)
fi

APP="target/release/bundle/macos/Keelwire.app"
echo
echo "Done."
echo "  App: $(pwd)/$APP"
ls "$(pwd)"/target/release/bundle/dmg/*.dmg 2>/dev/null | sed 's/^/  Disk image: /' || true
open "$APP"
