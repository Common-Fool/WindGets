"""Создаёт два значка трея и ресурсы приложения с одобренными оранжевыми плитками."""

import argparse
import base64
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


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


def make_widget_mosaic():
    image = canvas()
    draw = ImageDraw.Draw(image)
    for box in ((1, 1, 7, 15), (9, 1, 15, 7), (9, 9, 15, 15)):
        tile(draw, box, "#FF842B")
    return image


def make_mono_mosaic():
    image = canvas()
    draw = ImageDraw.Draw(image)
    # Чёрный край сохраняет белый силуэт на светлой панели Windows.
    for box in ((0.5, 0.5, 7.5, 15.5), (8.5, 0.5, 15.5, 7.5), (8.5, 8.5, 15.5, 15.5)):
        tile(draw, box, "#FFFFFF", 1.3, "#161616", 0.75)
    return image


ICONS = {
    "widget-mosaic": (make_widget_mosaic, "Плитки", "01"),
    "mono-mosaic": (make_mono_mosaic, "Монохром", "10"),
}


def bundle_icons():
    """У exe, установщика и бандла один силуэт, без старого графика."""
    source = make_widget_mosaic()
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


def preview(directory):
    directory.mkdir(parents=True, exist_ok=True)
    board = Image.new("RGB", (508, 364), "#151719")
    draw = ImageDraw.Draw(board)
    fonts = Path("C:/Windows/Fonts")
    regular = lambda size: ImageFont.truetype(str(fonts / "segoeui.ttf"), size)
    bold = lambda size: ImageFont.truetype(str(fonts / "segoeuib.ttf"), size)
    draw.text((28, 16), "Windgets · выбранные иконки", font=bold(24), fill="#FAF7F1")
    draw.text((28, 54), "Варианты 01 и 10 · тёмный и светлый фон", font=regular(15), fill="#AEB2B4")
    cards = []
    for index, (icon_id, (_, title, number)) in enumerate(ICONS.items()):
        x, y = 28 + index * 226, 94
        draw.rounded_rectangle((x, y, x + 210, y + 242), radius=14, fill="#222528", outline="#373A3D")
        draw.text((x + 14, y + 12), f"{number} · {title}", font=bold(17), fill="#FAF7F1")
        with Image.open(OUT / f"{icon_id}.png") as source:
            icon = source.resize((64, 64), Image.Resampling.LANCZOS)
        tray = Image.frombytes("RGBA", (32, 32), (OUT / f"{icon_id}.rgba").read_bytes())
        for j, color in enumerate(("#151719", "#ECEBE7")):
            draw.rounded_rectangle((x + 14 + j * 94, y + 56, x + 102 + j * 94, y + 144), radius=10, fill=color)
            board.paste(icon, (x + 26 + j * 94, y + 68), icon)
        draw.text((x + 14, y + 158), "Настоящий размер:", font=regular(13), fill="#AEB2B4")
        for j, size in enumerate((16, 24, 32)):
            small = tray.resize((size, size), Image.Resampling.LANCZOS)
            board.paste(small, (x + 26 + j * 64, y + 184 + (32 - size) // 2), small)
            draw.text((x + 24 + j * 64, y + 218), f"{size} px", font=regular(11), fill="#9DA3A6")
        data = base64.b64encode((OUT / f"{icon_id}.png").read_bytes()).decode("ascii")
        cards.append(f'<article><h2>{number} · {title}</h2><div class="samples"><span class="dark"><img src="data:image/png;base64,{data}"></span><span class="light"><img src="data:image/png;base64,{data}"></span></div><div class="native"><img style="width:16px;height:16px" src="data:image/png;base64,{data}"><img style="width:24px;height:24px" src="data:image/png;base64,{data}"><img style="width:32px;height:32px" src="data:image/png;base64,{data}"></div><small>16 px · 24 px · 32 px</small></article>')
    board.save(directory / "windgets-icons-selected.png", optimize=True)
    html = '<!doctype html><html lang="ru"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Windgets — выбранные иконки</title><style>body{margin:0;padding:32px;background:#151719;color:#eee;font:16px Segoe UI,sans-serif}h1{font-size:26px}main{display:flex;flex-wrap:wrap;gap:16px}article{padding:16px;border:1px solid #373a3d;border-radius:14px;background:#222528}h2{font-size:17px}img{width:64px;height:64px}.samples{display:flex;gap:8px}.samples span{width:88px;height:88px;display:grid;place-items:center;border-radius:10px}.dark{background:#151719}.light{background:#ecebe7}.native{display:flex;align-items:center;justify-content:space-between;margin-top:20px;min-height:32px}small{display:block;color:#aaa;margin-top:8px}</style><h1>Windgets · выбранные иконки</h1><main>' + "".join(cards) + '</main></html>'
    (directory / "windgets-icons-selected.html").write_text(html, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--preview-dir", type=Path)
    parser.add_argument("--bundle-only", action="store_true", help="Обновить только ресурсы exe/установщика")
    args = parser.parse_args()
    if args.bundle_only:
        bundle_icons()
        return
    OUT.mkdir(parents=True, exist_ok=True)
    for icon_id, (render, _, _) in ICONS.items():
        source = render()
        source.resize((SIZE, SIZE), Image.Resampling.LANCZOS).save(OUT / f"{icon_id}.png", optimize=True)
        # Поля и геометрия совпадают у PNG и трея, наружной подложки нет.
        tray = source.resize((TRAY_SIZE, TRAY_SIZE), Image.Resampling.LANCZOS)
        (OUT / f"{icon_id}.rgba").write_bytes(tray.tobytes())
    bundle_icons()
    if args.preview_dir:
        preview(args.preview_dir)
    print(f"Generated {len(ICONS)} icons in {OUT}")


if __name__ == "__main__":
    main()
