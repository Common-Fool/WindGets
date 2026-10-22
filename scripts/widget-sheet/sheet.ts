import { DEFAULT_ACCENT_COLOR, DEFAULT_CUSTOM_COLORS, type Design, type Look } from "../../src/lib/look";
import { EMPTY_SETTINGS, type SettingsView, type WidgetRow } from "../../src/lib/settings";
import { HEIGHT, READY, TO_FRAME, type ToFrame, type ToHost } from "../../src/preview/protocol";
import { PREVIEWED } from "../../src/preview/list";
import sizes from "./sizes.json";

/*
 * Лист снимков для README: живые превью всех виджетов на демоданных — те же
 * рамки, что в разделе «Виджеты» окна настроек (`src/preview/`).
 *
 * Открывается дев-сервером: `localhost:1420/scripts/widget-sheet/index.html`
 * с `?design=reference|app`, `?lang=en`, `?only=<id>` (один виджет),
 * `?columns=4`. Снимает его `shoot.mjs` рядом. Размеры окон —
 * `sizes.json`, его выгружает Rust:
 * `cargo test --lib widget_sheet_sizes -- --ignored`.
 */

const params = new URLSearchParams(window.location.search);
const design: Design = params.get("design") === "app" ? "app" : "reference";
const language = params.get("lang") ?? "en";
const only = params.get("only");
const columns = Number(params.get("columns") ?? 4);
const GAP = 16;

const table = (sizes as Record<Design, Record<string, [number, number]>>)[design];
const ids = only !== null ? [only] : PREVIEWED.filter((id) => id in table);

// Темы по умолчанию каждого дизайна — как у нового пользователя
const look: Look = {
  design,
  effectiveDesign: design,
  cardOpacity: EMPTY_SETTINGS.cardOpacity,
  scale: 1,
  palette: design === "reference" ? "gpt" : "reference-classic",
  glass: design === "reference" ? "gpt" : "obsidian",
  accentColor: DEFAULT_ACCENT_COLOR,
  customColors: DEFAULT_CUSTOM_COLORS,
  vivid: 1,
};

const rows: WidgetRow[] = ids.map((id) => ({
  id,
  title: id,
  service: false,
  enabled: true,
  alwaysOnTop: false,
  locked: false,
  view: 0,
  views: [],
  // Заметки по умолчанию во второй компоновке — как в `Settings::widget`
  layout: id === "notes" ? 1 : 0,
  layouts: [],
  cover: 0,
  covers: [],
  size: 0,
  sizes: [],
  flexible: false,
  cardOpacity: null,
  scale: null,
  preview: table[id] ?? [360, 240],
}));
const settings: SettingsView = { ...EMPTY_SETTINGS, design, widgets: rows };

const sheet = document.getElementById("sheet")!;
type Slot = { slot: HTMLDivElement; frame: HTMLIFrameElement; width: number; height: number };
type Placed = { left: number; top: number; entry: Slot };
const slots = new Map<string, Slot>();

for (const row of rows) {
  const [width, height] = row.preview;
  const slot = document.createElement("div");
  slot.className = "slot";
  const frame = document.createElement("iframe");
  frame.src = `/index.html?widget=${row.id}&preview`;
  slot.append(frame);
  sheet.append(slot);
  slots.set(row.id, { slot, frame, width, height });
}

/** Раскладка «в самый короткий столбец»: плотно, порядок реестра сохраняется. */
function layout(): void {
  const count = Math.max(1, Math.min(columns, slots.size));
  const tops = new Array<number>(count).fill(0);
  const placed: Placed[] = [];
  const assigned: Placed[][] = Array.from({ length: count }, () => []);

  for (const entry of slots.values()) {
    const column = tops.indexOf(Math.min(...tops));
    assigned[column]!.push({ left: 0, top: tops[column]!, entry });
    tops[column]! += entry.height + GAP;
  }

  let left = 24;
  for (const items of assigned) {
    const width = Math.max(0, ...items.map((item) => item.entry.width));
    for (const item of items) placed.push({ ...item, left });
    if (width > 0) left += width + GAP;
  }

  for (const { left: x, top, entry } of placed) {
    Object.assign(entry.slot.style, { left: `${x}px`, top: `${top + 24}px`, width: `${entry.width}px`, height: `${entry.height}px` });
    Object.assign(entry.frame.style, { width: `${entry.width}px`, height: `${entry.height}px` });
  }
  sheet.style.width = `${left - GAP - 24}px`;
  sheet.style.height = `${Math.max(...tops) - GAP}px`;
}

window.addEventListener("message", (event: MessageEvent<ToHost>) => {
  if (event.origin !== window.location.origin) return;
  const entry = [...slots.values()].find((item) => item.frame.contentWindow === event.source);
  if (entry === undefined) return;
  if (event.data?.type === READY) {
    const message: ToFrame = { type: TO_FRAME, state: { look, language, settings } };
    entry.frame.contentWindow?.postMessage(message, window.location.origin);
  }
  if (event.data?.type === HEIGHT && Number.isFinite(event.data.height)) {
    entry.height = Math.round(event.data.height);
    layout();
  }
});

layout();
