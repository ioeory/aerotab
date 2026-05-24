"""Regenerate Tauri bundle icons from docs/assets/logo.png."""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOGO = ROOT / "docs" / "assets" / "logo.png"
ICONS = ROOT / "src-tauri" / "icons"
TAURI_DIR = ROOT / "src-tauri"


def main() -> None:
    if not LOGO.is_file():
        raise SystemExit(f"Missing logo: {LOGO}")
    ICONS.mkdir(parents=True, exist_ok=True)
    shutil.copy2(LOGO, ICONS / "icon-source.png")
    rel_logo = LOGO.relative_to(TAURI_DIR)
    print(f"Generating icons from {rel_logo} …")
    subprocess.run(
        ["cargo", "tauri", "icon", str(rel_logo)],
        cwd=TAURI_DIR,
        check=True,
    )
    print("Done. Icons written to src-tauri/icons/")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        raise SystemExit(e.returncode) from e
