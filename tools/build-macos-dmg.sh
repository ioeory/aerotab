#!/usr/bin/env bash
# Build AeroTab macOS .dmg (must run on macOS; can cross Intel↔ARM via TARGET).
# Usage (repo root): ./tools/build-macos-dmg.sh [extra cargo tauri args...]
# Env:
#   BUNDLES   Tauri bundle list (default: dmg). Example: BUNDLES=app,dmg
#   FEATURES  Optional cargo features (e.g. FEATURES=desktop)
#   TARGET    Rust triple override. Examples:
#               x86_64-apple-darwin   — Intel DMG from Apple Silicon
#               aarch64-apple-darwin  — ARM DMG (usually native on M-series)
#   TAURI_BUNDLER_DMG_IGNORE_CI  Set to true in CI to skip Finder AppleScript layout
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

bundles="${BUNDLES:-dmg}"

# --- Preconditions -----------------------------------------------------------

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS .dmg must be built on macOS; got $(uname -s)" >&2
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
  echo "pkg-config is required: brew install pkg-config" >&2
  exit 1
fi

if ! command -v create-dmg >/dev/null 2>&1; then
  echo "create-dmg is required: brew install create-dmg" >&2
  exit 1
fi

if ! xcode-select -p >/dev/null 2>&1; then
  echo "Xcode Command Line Tools are required: xcode-select --install" >&2
  exit 1
fi

# --- Resolve target / artifact path ------------------------------------------

version=""
if command -v jq >/dev/null 2>&1; then
  version="$(jq -r '.version // empty' src-tauri/tauri.conf.json 2>/dev/null || true)"
fi

host="$(uname -m)"
target="${TARGET:-}"
tauri_args=(--bundles "${bundles}")

if [[ -n "${FEATURES:-}" ]]; then
  tauri_args+=(--features "${FEATURES}")
fi

if [[ -n "${target}" ]]; then
  rustup target add "${target}" >/dev/null
  tauri_args+=(--target "${target}")
  out_root="target/${target}/release"
else
  out_root="target/release"
fi

# DMG suffix used by the Tauri bundler: Intel → x64, Apple Silicon → aarch64.
effective="${target}"
if [[ -z "${effective}" ]]; then
  case "${host}" in
    x86_64) effective="x86_64-apple-darwin" ;;
    arm64 | aarch64) effective="aarch64-apple-darwin" ;;
  esac
fi
case "${effective}" in
  x86_64-apple-darwin) dmg_suffix="x64" ;;
  aarch64-apple-darwin) dmg_suffix="aarch64" ;;
  *) dmg_suffix="" ;;
esac

echo "Building AeroTab${version:+ ${version}} .dmg (host=${host}${target:+ target=${target}}, bundles=${bundles})..."

(
  cd src-tauri
  cargo tauri build "${tauri_args[@]}" "$@"
)

# --- Locate artifact ---------------------------------------------------------

shopt -s nullglob
dmgs=()
if [[ -n "${dmg_suffix}" ]]; then
  dmgs=("${out_root}/bundle/dmg/AeroTab_"*"_${dmg_suffix}.dmg")
fi
if ((${#dmgs[@]} == 0)); then
  dmgs=("${out_root}/bundle/dmg/AeroTab_"*.dmg)
fi
# Native ARM sometimes still lands under the explicit triple path.
if ((${#dmgs[@]} == 0)) && [[ -z "${target}" ]]; then
  dmgs=("target/aarch64-apple-darwin/release/bundle/dmg/AeroTab_"*.dmg)
  dmgs+=("target/x86_64-apple-darwin/release/bundle/dmg/AeroTab_"*.dmg)
fi
if ((${#dmgs[@]} == 0)); then
  # shellcheck disable=SC2207
  dmgs=($(find target -type f -path '*/bundle/dmg/AeroTab_*.dmg' 2>/dev/null | sort))
fi
shopt -u nullglob

if ((${#dmgs[@]} == 0)); then
  echo "build finished but no AeroTab_*.dmg found under target/" >&2
  find target -type f -name '*.dmg' 2>/dev/null || true
  exit 1
fi

ls -la "${dmgs[@]}"
echo "OK: ${dmgs[0]}"
