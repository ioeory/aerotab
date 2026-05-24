"""Generate AeroTab app icons from icon-source.png (32, 128, 256 px)."""

from __future__ import annotations

from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parents[1] / "src-tauri" / "icons"
SOURCE = OUT / "icon-source.png"
SIZES = (32, 128, 256)


def main() -> None:
    if not SOURCE.is_file():
        raise SystemExit(f"Missing source icon: {SOURCE}")
    src = Image.open(SOURCE).convert("RGBA")
    OUT.mkdir(parents=True, exist_ok=True)
    for size in SIZES:
        img = src.resize((size, size), Image.Resampling.LANCZOS)
        out = OUT / ("icon.png" if size == 256 else f"{size}x{size}.png")
        img.save(out, format="PNG")
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
