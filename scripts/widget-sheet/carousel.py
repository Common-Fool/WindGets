"""Листалка виджетов для README: анимированный WebP из снимков по одному.

Карусели в README GitHub не бывает (скриптов там нет), поэтому виджеты
сменяют друг друга сами. WebP, а не GIF: у GIF 256 цветов, и стекло с
градиентами пошло бы полосами; GitHub проигрывает WebP так же.

Берёт снимки, которые сделал `shoot.mjs` (`assets/screenshots/<id>.webp`, GPT
Design), и пишет `assets/screenshots/widgets.webp`. Нужен Pillow:
`python scripts/widget-sheet/carousel.py`.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
SHOTS = ROOT / "assets" / "screenshots"
OUT = SHOTS / "widgets.webp"

# Порядок и подписи — те же, что у галереи в README
WIDGETS = [
    ("gpu", "GPU"),
    ("cpu", "CPU and RAM"),
    ("disks", "Drives"),
    ("network", "Network"),
    ("processes", "Processes"),
    ("battery", "Battery"),
    ("speedtest", "Speedtest"),
    ("weather", "Weather"),
    ("sound", "Sound"),
    ("spotify", "Spotify"),
    ("media", "Media"),
    ("mail", "Mail"),
    ("totp", "2FA codes"),
    ("notes", "Notes"),
    ("todo", "To-do"),
    ("clipboard", "Clipboard history"),
    ("password", "Password generator"),
    ("converter", "Converter"),
    ("downloader", "Downloader"),
    ("fps", "Frame counter"),
]

# Снимки сняты с плотностью точек 2; кадр уменьшается до 0.6 — чётко и легко
SCALE = 0.6
# Поле подложки листа вокруг карточки (24 точки × 2) и скругление окна (8 × 2):
# подложку срезаем — её градиент не совпал бы с фоном кадра и читался рамкой
PAD = 48
RADIUS = 16
MARGIN = 48
CAPTION = 64
FRAME_MS = 2200
TOP, BOTTOM = (22, 27, 32), (12, 15, 18)


def backdrop(width: int, height: int) -> Image.Image:
    """Та же тёмная подложка, что у листа: вертикальный градиент."""
    image = Image.new("RGB", (width, height))
    draw = ImageDraw.Draw(image)
    for y in range(height):
        t = y / max(1, height - 1)
        draw.line([(0, y), (width, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    return image


def card(path: Path) -> Image.Image:
    """Карточка без подложки листа, со скруглёнными, как у окна, углами."""
    shot = Image.open(path).convert("RGBA")
    shot = shot.crop((PAD, PAD, shot.width - PAD, shot.height - PAD))
    mask = Image.new("L", shot.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, shot.width - 1, shot.height - 1), radius=RADIUS, fill=255)
    shot.putalpha(Image.composite(shot.getchannel("A"), mask, mask))
    return shot


def main() -> None:
    shots = [(card(SHOTS / f"{id}.webp"), title) for id, title in WIDGETS]
    shots = [(shot.resize((round(shot.width * SCALE), round(shot.height * SCALE)), Image.Resampling.LANCZOS), title) for shot, title in shots]
    width = max(shot.width for shot, _ in shots) + MARGIN * 2
    height = max(shot.height for shot, _ in shots) + MARGIN * 2 + CAPTION
    font = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 26)
    small = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 18)

    frames = []
    for index, (shot, title) in enumerate(shots):
        frame = backdrop(width, height)
        area = height - CAPTION
        frame.paste(shot, ((width - shot.width) // 2, (area - shot.height) // 2), shot)
        draw = ImageDraw.Draw(frame)
        draw.text((width // 2, area + 8), title, font=font, fill=(236, 240, 242), anchor="mt")
        draw.text((width // 2, area + 42), f"{index + 1} / {len(shots)}", font=small, fill=(130, 140, 148), anchor="mt")
        frames.append(frame)

    frames[0].save(OUT, save_all=True, append_images=frames[1:], duration=FRAME_MS, loop=0, quality=88, method=6)
    print(f"{OUT.name}: {width}x{height}, {len(frames)} кадров, {OUT.stat().st_size // 1024} КБ")


if __name__ == "__main__":
    main()
