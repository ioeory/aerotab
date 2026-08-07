#!/usr/bin/env bash
# Build AeroTab Linux .deb (native only — GTK/WebKit cannot be cross-compiled).
# Usage (repo root): ./tools/build-linux-deb.sh [extra cargo tauri args...]
# Env:
#   BUNDLES   Tauri bundle list (default: deb). Example: BUNDLES=deb,appimage
#   FEATURES  Cargo features (default: desktop)
#   SKIP_NPM_INSTALL  Set to 1 to skip the apps/ui dependency install
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

bundles="${BUNDLES:-deb}"
features="${FEATURES:-desktop}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Linux .deb must be built on Linux (native GTK/WebKit); got $(uname -s)" >&2
  echo "Use a Debian/Ubuntu host, CI runner, or container — not macOS/Windows." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required (install via https://rustup.rs)" >&2
  exit 1
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  echo "tauri-cli is required: cargo install tauri-cli --locked" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "node is required (Node 20+)" >&2
  exit 1
fi

if ! command -v pkg-config >/dev/null 2>&1; then
  echo "pkg-config is required (e.g. sudo apt install pkg-config)" >&2
  exit 1
fi

missing=()
for pc in openssl libudev webkit2gtk-4.1; do
  if ! pkg-config --exists "$pc" 2>/dev/null; then
    missing+=("$pc")
  fi
done
if ((${#missing[@]} > 0)); then
  echo "missing pkg-config packages: ${missing[*]}" >&2
  echo "Debian/Ubuntu example:" >&2
  cat >&2 <<'APT'
sudo apt-get install -y \
  libssl-dev pkg-config libudev-dev \
  libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
  libwebkit2gtk-4.1-dev file wget patchelf
APT
  exit 1
fi

# --- Frontend dependencies ----------------------------------------------------

# `beforeBuildCommand` runs `npx vite build`, which fails outright when
# apps/ui/node_modules is missing.
ensure_frontend_deps() {
  if [[ "${SKIP_NPM_INSTALL:-0}" == "1" ]]; then
    return 0
  fi
  if [[ -d apps/ui/node_modules/vite ]]; then
    return 0
  fi
  echo "Installing apps/ui dependencies..."
  (
    cd apps/ui
    if [[ -f package-lock.json ]]; then
      npm ci --no-audit --no-fund
    else
      npm install --no-audit --no-fund
    fi
  )
}

ensure_frontend_deps

# --- Build -------------------------------------------------------------------

version=""
if command -v jq >/dev/null 2>&1; then
  version="$(jq -r '.version // empty' src-tauri/tauri.conf.json 2>/dev/null || true)"
fi
arch="$(uname -m)"
case "${arch}" in
  x86_64) deb_arch="amd64" ;;
  aarch64 | arm64) deb_arch="arm64" ;;
  *)
    echo "unsupported architecture: ${arch} (expected x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

echo "Building AeroTab${version:+ ${version}} .deb for ${arch} (bundles=${bundles})..."

(
  cd src-tauri
  cargo tauri build \
    --bundles "${bundles}" \
    --features "${features}" \
    "$@"
)

shopt -s nullglob
debs=(target/release/bundle/deb/AeroTab_*_"${deb_arch}".deb)
shopt -u nullglob

if ((${#debs[@]} == 0)); then
  echo "build finished but no .deb matched AeroTab_*_${deb_arch}.deb" >&2
  find target/release/bundle -type f -name '*.deb' 2>/dev/null || true
  exit 1
fi

ls -la "${debs[@]}"
echo "OK: ${debs[0]}"
