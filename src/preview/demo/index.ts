import { DEFAULT_ACCENT_COLOR, DEFAULT_CUSTOM_COLORS, type Look } from "../../lib/look";
import { EMPTY_SETTINGS, type SettingsView, type WidgetRow } from "../../lib/settings";
import type { PreviewState } from "../protocol";
import { hardware } from "./hardware";
import { system } from "./system";
import { text } from "./text";
import { online } from "./online";
import { tools } from "./tools";
import { DEMO_PLACE } from "./common";
import { PREVIEWED } from "../list";
import type { Answers, Context } from "./common";

/*
 * Демоданные превью: ответы на вопросы, которые карточки задают Rust.
 *
 * ВАЖНО: **данные только нейтральные** — они уходят в публичную версию: ни
 * железа, ни города, ни ящиков автора. Железо — нейтральные примеры
 * (Ryzen 7 7700X, RTX 5070, NCT6799D), город — Грац, адреса — `example.test`,
 * обложек и логотипов нет.
 *
 * Ответы типизированы типами хранилищ из `src/lib`: правка формата снимка
 * ломает сборку, а не превью молча. Вопрос без ответа получает `null` — так
 * отвечают действия (нажать, перетащить, открыть меню), которых у превью нет.
 */

const ANSWERS: Answers = {
  ...hardware,
  ...system,
  ...text,
  ...online,
  ...tools,

  appearance: (_args, { id, state }) => lookOf(id, state),
  set_design: (_args, { id, state }) => lookOf(id, state),
  language: (_args, { state }) => state.language,
  settings_view: (_args, { state }): SettingsView => ({ ...state.settings, weatherPlace: DEMO_PLACE }),
  // Окно настроек в браузере (`?widget=settings&preview`) рисует страницу, только
  // когда видно
  "plugin:window|is_visible": () => true,
};

export function answer(command: string, args: Record<string, unknown>, context: Context): unknown {
  return ANSWERS[command]?.(args, context);
}

/**
 * Вид карточки в превью: тема, стекло и цвета — как у всех окон, масштаб —
 * 100 % (рамку уменьшает окно настроек), плотность — своя у виджета или общая.
 */
function lookOf(id: string, state: PreviewState): Look {
  const row = state.settings.widgets.find((widget) => widget.id === id);
  return { ...state.look, scale: 1, cardOpacity: row?.cardOpacity ?? state.settings.cardOpacity };
}

/**
 * Без рамки (проверка в браузере): одна строка виджета. Дизайн — из адреса
 * (`&design=app` — Claude, иначе GPT), вид — `&view=1`.
 */
export function defaultState(id: string): PreviewState {
  const query = new URLSearchParams(window.location.search);
  const design = query.get("design") === "app" ? "app" : "reference";
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
  const row: WidgetRow = {
    id,
    title: id,
    service: false,
    enabled: true,
    alwaysOnTop: false,
    locked: false,
    view: Number(query.get("view") ?? 0),
    views: [],
    layout: 0,
    layouts: [],
    cover: 0,
    covers: [],
    size: 0,
    sizes: [],
    flexible: false,
    cardOpacity: null,
    scale: null,
    preview: [360, 240],
  };
  const language = query.get("lang") ?? "ru";
  // Само окно настроек — проверка галереи в браузере: строка на каждый виджет
  const rows = id === "settings"
    ? PREVIEWED.map((widget, index) => ({ ...row, id: widget, title: widget, enabled: index < 6 }))
    : [row];
  return { look, language, settings: { ...EMPTY_SETTINGS, design, widgets: rows } };
}
