#!/usr/bin/env bash
# Copy NSIS installer to Windows Downloads, silent-install, launch for smoke test.
# Run from repository root after ./tools/build-windows-xwin.sh
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

version="$(grep -E '^version = ' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
installer_name="AeroTab_${version}_x64-setup.exe"
src="target/x86_64-pc-windows-msvc/release/bundle/nsis/${installer_name}"

if [[ ! -f "$src" ]]; then
  echo "Installer not found: $src" >&2
  echo "Run ./tools/build-windows-xwin.sh first." >&2
  exit 1
fi

win_user="${WIN_USER:-ioe}"
dest_dir="/mnt/c/Users/${win_user}/Downloads"
dest="${dest_dir}/AeroTab_${version}_x64-setup.exe"

mkdir -p "$dest_dir"
cp -f "$src" "$dest"
echo "Copied to ${dest}"

win_dest=$(wslpath -w "$dest" 2>/dev/null || echo "C:\\Users\\${win_user}\\Downloads\\AeroTab_${version}_x64-setup.exe")

# NSIS silent install (/S). Overwrites same-version install under LocalAppData\Programs.
powershell.exe -NoProfile -Command "
  \$ErrorActionPreference = 'Stop'
  \$installer = '${win_dest}'
  Write-Host \"Installing: \$installer\"
  \$p = Start-Process -FilePath \$installer -ArgumentList '/S' -Wait -PassThru
  if (\$p.ExitCode -ne 0) { throw \"Installer exit code \$(\$p.ExitCode)\" }
  \$candidates = @(
    (Join-Path \$env:LOCALAPPDATA 'AeroTab\aerotab-app.exe'),
    (Join-Path \$env:LOCALAPPDATA 'Programs\AeroTab\aerotab-app.exe'),
    (Join-Path \$env:LOCALAPPDATA 'Programs\aerotab\aerotab-app.exe')
  )
  \$exe = \$candidates | Where-Object { Test-Path \$_ } | Select-Object -First 1
  if (-not \$exe) {
    \$exe = Get-ChildItem -Path \$env:LOCALAPPDATA -Recurse -Filter 'aerotab-app.exe' -ErrorAction SilentlyContinue |
      Where-Object { \$_.FullName -match 'AeroTab' } |
      Sort-Object LastWriteTime -Descending |
      Select-Object -First 1 -ExpandProperty FullName
  }
  if (-not \$exe) { throw 'Installed aerotab-app.exe not found under LocalAppData' }
  Write-Host \"Launching: \$exe\"
  \$app = Start-Process -FilePath \$exe -PassThru
  Start-Sleep -Seconds 4
  \$proc = Get-Process -Id \$app.Id -ErrorAction SilentlyContinue
  if (-not \$proc) { throw 'Process exited within 4s' }
  Write-Host \"Smoke OK: PID \$(\$proc.Id) still running\"
"

echo "Windows smoke test passed for ${version}"
