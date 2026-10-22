"""Создаёт значок приложения — белые плитки с тёмной каймой — для трея, окон, exe и установщика."""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src" / "assets" / "app-icons"
SCALE = 4
SIZE = 128
TRAY_SIZE = 32
CANVAS = SIZE * SCALE
CLEAR = (0, 0, 0, 0)


def px(value):
    return int(round(value * SCALE))


def canvas():
    return Image.new("RGBA", (CANVAS, CANVAS), CLEAR)


def tile(draw, box, fill, radius=1, outline=None, width=1):
    coords = tuple(px(value * 8) for value in box)
    coords = (*coords[:2], coords[2] - 1, coords[3] - 1)
    draw.rounded_rectangle(
        coords, radius=px(radius * 8), fill=fill, outline=outline, width=px(width * 8)
    )


def make_mono_mosaic():
    image = canvas()
    draw = ImageDraw.Draw(image)
    # Чёрный край сохраняет белый силуэт на светлой панели Windows.
    for box in ((0.5, 0.5, 7.5, 15.5), (8.5, 0.5, 15.5, 7.5), (8.5, 8.5, 15.5, 15.5)):
        tile(draw, box, "#FFFFFF", 1.3, "#161616", 0.75)
    return image


# Значок один на всё (22 октября, слово пользователя): оранжевые «Плитки» и
# выбор в настройках убраны. Имя файла осталось прежним — его берут Rust и трей.
ICON_ID = "mono-mosaic"


def bundle_icons():
    """У exe, установщика и бандла тот же силуэт, что в трее."""
    source = make_mono_mosaic()
    directory = ROOT / "src-tauri" / "icons"
    directory.mkdir(parents=True, exist_ok=True)
    (ROOT / "assets").mkdir(parents=True, exist_ok=True)
    outputs = {
        ROOT / "assets" / "icon.png": 1024,
        directory / "icon.png": 1024,
        directory / "32x32.png": 32,
        directory / "64x64.png": 64,
        directory / "128x128.png": 128,
        directory / "128x128@2x.png": 256,
    }
    for path, size in outputs.items():
        source.resize((size, size), Image.Resampling.LANCZOS).save(path, optimize=True)
    source.save(directory / "icon.ico", format="ICO",
                sizes=[(size, size) for size in (16, 24, 32, 48, 64, 128, 256)])
    print(f"Generated application icons in {directory}")


def preview(directory, source):
    """Доска для глаза: значок на тёмном и светлом фоне и в настоящих размерах."""
    directory.mkdir(parents=True, exist_ok=True)
    board = Image.new("RGB", (300, 220), "#151719")
    draw = ImageDraw.Draw(board)
    icon = source.resize((64, 64), Image.Resampling.LANCZOS)
    tray = Image.frombytes("RGBA", (32, 32), (OUT / f"{ICON_ID}.rgba").read_bytes())
    for j, color in enumerate(("#151719", "#ECEBE7")):
        draw.rounded_rectangle((20 + j * 100, 20, 108 + j * 100, 108), radius=10, fill=color)
        board.paste(icon, (32 + j * 100, 32), icon)
    for j, size in enumerate((16, 24, 32)):
        small = tray.resize((size, size), Image.Resampling.LANCZOS)
        board.paste(small, (32 + j * 64, 150 + (32 - size) // 2), small)
    board.save(directory / "windgets-icon.png", optimize=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--preview-dir", type=Path)
    parser.add_argument("--bundle-only", action="store_true", help="Обновить только ресурсы exe/установщика")
    args = parser.parse_args()
    if args.bundle_only:
        bundle_icons()
        return
    OUT.mkdir(parents=True, exist_ok=True)
    source = make_mono_mosaic()
    # Трей и окна берут сырые байты 32×32, PNG им не нужен
    tray = source.resize((TRAY_SIZE, TRAY_SIZE), Image.Resampling.LANCZOS)
    (OUT / f"{ICON_ID}.rgba").write_bytes(tray.tobytes())
    bundle_icons()
    if args.preview_dir:
        preview(args.preview_dir, source)
    print(f"Generated {ICON_ID} in {OUT}")


if __name__ == "__main__":
    main()
