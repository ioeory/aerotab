"""Generate Tabby v2 app icons at 32, 128, and 256 px.

Design: rounded-square dark background (#0d1117) with a centered ">_"
prompt glyph drawn in accent blue (#58a6ff). Anti-aliased via 4x
supersampling.
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

BG = (13, 17, 23, 255)        # #0d1117
ACCENT = (88, 166, 255, 255)  # #58a6ff
BORDER = (48, 54, 61, 255)    # #30363d

OUT = Path(__file__).resolve().parents[1] / "src-tauri" / "icons"


def find_mono_font(size: int) -> ImageFont.ImageFont:
    candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
    ]
    for p in candidates:
        try:
            return ImageFont.truetype(p, size=size)
        except OSError:
            continue
    return ImageFont.load_default()


def make_icon(size: int) -> Image.Image:
    scale = 4
    s = size * scale
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    radius = int(s * 0.22)
    draw.rounded_rectangle((0, 0, s - 1, s - 1), radius=radius, fill=BG, outline=BORDER, width=max(2, scale))

    # Glyph ">_" centered.
    font = find_mono_font(int(s * 0.55))
    text = ">_"
    bbox = draw.textbbox((0, 0), text, font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    x = (s - tw) // 2 - bbox[0]
    y = (s - th) // 2 - bbox[1] - int(s * 0.02)
    draw.text((x, y), text, font=font, fill=ACCENT)

    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for px, name in [(32, "32x32.png"), (128, "128x128.png"), (256, "icon.png")]:
        path = OUT / name
        make_icon(px).save(path, "PNG")
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
