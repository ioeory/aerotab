#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

target="${TARGET:-x86_64-pc-windows-msvc}"
bundles="${BUNDLES:-nsis}"

if ! command -v cargo-xwin >/dev/null 2>&1; then
  echo "cargo-xwin is required: cargo install cargo-xwin" >&2
  exit 1
fi

if ! command -v makensis >/dev/null 2>&1; then
  echo "makensis is required for NSIS bundles" >&2
  exit 1
fi

rustup target add "$target" >/dev/null

(
  cd src-tauri
  cargo tauri build \
    --runner cargo-xwin \
    --target "$target" \
    --bundles "$bundles" \
    "$@"
)