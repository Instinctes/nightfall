#!/usr/bin/env python3
"""Every NIGHTFALLCOIN logo and icon, from one square master PNG.

    python3 scripts/make-brand-assets.py "New Logo.png"

Writes, relative to the repository root (only paths whose directory exists):

  assets/brand/            icon set: PNG 16-1024, favicon.ico, AppIcon.icns,
                           (icon-1024.png is the reference copy kept in git)
  assets/                  logo.png (256), AppIcon.icns, AppIcon.ico
  crates/nightfall-core/assets/
                           logo.png (256, sidebar + tray), logo-512.png (window)
  mobile/ios/.../AppIcon.png
                           1024, on black: iOS app icons may not be transparent
  website/public/assets/   favicon-32, apple-touch-icon (180), logo-128,
                           logo-256, logo.png (512), og.png (1200x630 card)
  website/public/wallet-origin/
                           favicon-32, apple-touch-icon, icon-192, icon-512,
                           logo-128

Transparent corners everywhere else, as before. Downscaling is done in one
step from the master with Lanczos, never from an already reduced copy.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
OG_BACKGROUND = (20, 16, 34)   # the site's night background, as in the old card
OG_SIZE = (1200, 630)
OG_LOGO = 337                  # canvas size; the ring itself is ~300 px, as before
OG_CENTER_Y = 270


def load_master(path: Path) -> Image.Image:
    im = Image.open(path).convert("RGBA")
    if im.width != im.height:
        sys.exit(f"master must be square, got {im.size}")
    if im.width < 1024:
        sys.exit(f"master must be at least 1024 px, got {im.width}")
    return im


def sized(master: Image.Image, px: int) -> Image.Image:
    return master.resize((px, px), Image.LANCZOS)


def save(img: Image.Image, rel: str, written: list) -> None:
    out = ROOT / rel
    if not out.parent.is_dir():
        return
    img.save(out, optimize=True)
    written.append(rel)


def icns(master: Image.Image, rel: str, written: list) -> None:
    out = ROOT / rel
    if not out.parent.is_dir() or shutil.which("iconutil") is None:
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "AppIcon.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            sized(master, size).save(iconset / f"icon_{size}x{size}.png")
            sized(master, size * 2).save(iconset / f"icon_{size}x{size}@2x.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(out)], check=True)
    written.append(rel)


def ico(master: Image.Image, rel: str, sizes, written: list) -> None:
    out = ROOT / rel
    if not out.parent.is_dir():
        return
    biggest = max(sizes)
    sized(master, biggest).save(out, format="ICO", sizes=[(s, s) for s in sizes])
    written.append(rel)


def og_card(master: Image.Image) -> Image.Image:
    card = Image.new("RGBA", OG_SIZE, OG_BACKGROUND + (255,))
    logo = sized(master, OG_LOGO)
    card.alpha_composite(logo, ((OG_SIZE[0] - OG_LOGO) // 2, OG_CENTER_Y - OG_LOGO // 2))
    return card.convert("RGB")


def on_black(img: Image.Image) -> Image.Image:
    base = Image.new("RGBA", img.size, (0, 0, 0, 255))
    base.alpha_composite(img)
    return base.convert("RGB")


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    master = load_master(Path(sys.argv[1]))
    w: list = []

    brand = ROOT / "assets" / "brand"
    brand.mkdir(parents=True, exist_ok=True)
    for px in (16, 32, 48, 64, 128, 180, 192, 256, 512, 1024):
        save(sized(master, px), f"assets/brand/icon-{px}.png", w)
    ico(master, "assets/brand/favicon.ico", (16, 32, 48), w)
    icns(master, "assets/brand/AppIcon.icns", w)

    save(sized(master, 256), "assets/logo.png", w)
    icns(master, "assets/AppIcon.icns", w)
    ico(master, "assets/AppIcon.ico", (16, 24, 32, 48, 64, 128, 256), w)

    save(sized(master, 256), "crates/nightfall-core/assets/logo.png", w)
    save(sized(master, 512), "crates/nightfall-core/assets/logo-512.png", w)

    save(on_black(sized(master, 1024)),
         "mobile/ios/NightfallWallet/Assets.xcassets/AppIcon.appiconset/AppIcon.png", w)

    site = "website/public/assets"
    save(sized(master, 32), f"{site}/favicon-32.png", w)
    save(sized(master, 180), f"{site}/apple-touch-icon.png", w)
    save(sized(master, 128), f"{site}/logo-128.png", w)
    save(sized(master, 256), f"{site}/logo-256.png", w)
    save(sized(master, 512), f"{site}/logo.png", w)
    save(og_card(master), f"{site}/og.png", w)

    wallet = "website/public/wallet-origin"
    save(sized(master, 32), f"{wallet}/favicon-32.png", w)
    save(sized(master, 180), f"{wallet}/apple-touch-icon.png", w)
    save(sized(master, 192), f"{wallet}/icon-192.png", w)
    save(sized(master, 512), f"{wallet}/icon-512.png", w)
    save(sized(master, 128), f"{wallet}/logo-128.png", w)

    for rel in w:
        print("wrote", rel)


if __name__ == "__main__":
    main()
