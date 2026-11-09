#!/usr/bin/env bash
# Safe by default: isolate startup and use a network-disabled Sandbox when a user app runs.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
version="$(node -p "require('./apps/ui/package.json').version")"
installer_name="AeroTab_${version}_x64-setup.exe"
src="target/x86_64-pc-windows-msvc/release/bundle/nsis/${installer_name}"
[[ -f "$src" ]] || { echo "Installer not found: $src" >&2; exit 1; }
[[ "${SMOKE_ISOLATED:-1}" == 1 ]] || { echo 'Only isolated smoke is supported; refusing real user startup.' >&2; exit 1; }
win_user="${WIN_USER:-ioe}"
dest_dir="/mnt/c/Users/${win_user}/Downloads/aerotab-smoke-${version}"
mkdir -p "$dest_dir"
cp -f "$src" "$dest_dir/$installer_name"
cp -f tools/windows-isolated-smoke.ps1 "$dest_dir/windows-isolated-smoke.ps1"
echo "Copied to $dest_dir/$installer_name"
# The PowerShell helper passes --smoke-test, checks MainWindowTitle including
# [SMOKE TEST - ISOLATED], and installs with /D= in a separate temporary directory.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w "$dest_dir/windows-isolated-smoke.ps1")" -Version "$version"
