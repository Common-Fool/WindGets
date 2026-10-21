import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Language } from "./i18n";
import { lookSettled } from "./look";

/*
 * Настройки приложения для их собственного окна.
 *
 * Устроено как у спидтеста и пульта: команда отдаёт снимок при открытии,
 * дальше он приезжает событием. Менять настройки может не только это окно —
 * рубильники дублированы в трее, а крестик на отладочном окне выключает
 * виджет сам, — поэтому источник правды один и лежит в Rust.
 *
 * Плотность фона и кегль сюда не входят: у них своё событие и своё
 * хранилище (`look.ts`), общее на все окна, а не на одно это.
 */

/** Столбцы в списке виджетов, кроме «Вкл.»: у того своя команда. */
export type WidgetOption = "top" | "through" | "lock";

/** Рубильники, продублированные в трее и в разделе «Окна». */
export type Flag = "hidden_all" | "locked" | "snapping" | "autostart";

import type { Design } from "./look";

export type WidgetRow = {
  id: string;
  title: string;
  /** Виджет нажимают: прозрачности для мыши у него не бывает. */
  interactive: boolean;
  /** Служебное окно: галочки «Все» его не трогают. */
  service: boolean;
  enabled: boolean;
  alwaysOnTop: boolean;
  clickThrough: boolean;
  /** Окно заперто поштучно. Общий рубильник этого флага не меняет. */
  locked: boolean;
  /** Номер вида: по нему карточка решает, чем себя рисовать. */
  view: number;
  /** Имена видов. Пусто или одна запись — переключать нечего. */
  views: string[];
  layout: number;
  layouts: string[];
  /** Номер способа показать обложку. */
  cover: number;
  /** Имена способов. Пусто — обложки у виджета нет вовсе. */
  covers: string[];
  /** Номер готового размера. */
  size: number;
  /** Имена готовых размеров: «Полный», «Без истории», «Только полоса». */
  sizes: string[];
  /** Номера пресетов не перенумеровываются при фильтрации по дизайну. */
  sizeIndices?: number[];
  /** Окно тянется мышью, готовых размеров у него нет. */
  flexible: boolean;
  /** Своя плотность фона. `null` — как у всех. */
  cardOpacity: number | null;
  /** Свой масштаб. `null` — как у всех. */
  scale: number | null;
};

/** Город виджета погоды: имя для показа и координаты для запроса. */
export type Place = {
  name: string;
  /** «Штирия, Австрия» — уточнение под именем. */
  region: string;
  latitude: number;
  longitude: number;
};

/** Один пункт списка выбора — тема или стекло. */
export type ChoiceRow = {
  id: string;
  title: string;
};

export type SettingsView = {
  widgets: WidgetRow[];
  cardOpacity: number;
  scale: number;
  /**
   * Все темы: имена для карточек приходят из Rust, а не пишутся тут. Выбранную
   * окно знает из вида (`look.ts`), как и все остальные окна.
   */
  palettes: ChoiceRow[];
  /** Все стёкла — по той же причине. */
  glasses: ChoiceRow[];
  /** Яркость цветов. */
  vivid: number;
  autostart: boolean;
  hiddenAll: boolean;
  locked: boolean;
  snapping: boolean;
  pollSeconds: number;
  /** `null` — город не выбран, и виджет погоды в сеть не ходит вовсе. */
  weatherPlace: Place | null;
  /** Явное сворачивание приложений в мини-микшере звука. */
  soundMixerCollapsed: boolean;
  /** Общий дизайн не меняет раскладки виджетов. */
  design: Design;
  /** Выбранный язык. `null` — «как в Windows». */
  language: Language | null;
  /** Выбранная иконка системного трея. */
  appIcon: string;
  /** Все языки самоназваниями: `id` — код, `title` — «Русский», «中文». */
  languages: ChoiceRow[];
  /** Язык Windows — что выберется при «как в Windows». */
  systemLanguage: Language;
  /**
   * Накопители, скрытые в карточке, — именами из выхлопа. Скрытые, а не
   * показанные: новый диск появляется в карточке сам.
   */
  hiddenDisks: string[];
  /**
   * Ряды, снятые с карточек процессора и видеокарты (этап 58): `cpu` и `gpu`
   * → снятые ряды. Нет ключа — показано всё.
   */
  hiddenMetrics: Partial<Record<string, Metric[]>>;
  /**
   * Ручной выбор датчиков платы: место → `off` или путь датчика без имени
   * машины. Нет ключа — автоматика.
   */
  sensorPicks: Partial<Record<SensorSlot, string>>;
  /** Версия приложения из `Cargo.toml`. Пусто — Rust ещё не ответил. */
  version: string;
  /** «Напоминать реже» у напоминания о донатах: раз в полгода, а не в месяц. */
  supportRare: boolean;
  /** Последний снимок датчиков сказал, что дискретной видеокарты нет. */
  noGpu: boolean;
  /** `settings.json` не прочитался при запуске: правки не сохраняются. */
  unsaved: boolean;
};

/** Ряды карточек процессора и видеокарты, которые можно снять. Повторяет `settings::METRICS`. */
export const METRICS = ["temperature", "fan", "clock", "power", "voltage"] as const;
export type Metric = (typeof METRICS)[number];

/** Где датчик платы выбирают руками. Повторяет `settings::SENSOR_SLOTS`. */
export type SensorSlot = "cpuFan" | "chassisFan" | "boardTemp" | "cpuVoltage";

/** «Не показывать» в ручном выборе — то же слово, что `settings::PICK_OFF`. */
export const PICK_OFF = "off";

/** До первого ответа Rust: пустой список, рубильники опущены. */
const EMPTY: SettingsView = {
  widgets: [],
  cardOpacity: 0.94,
  scale: 1,
  palettes: [],
  glasses: [],
  vivid: 1,
  autostart: false,
  hiddenAll: false,
  locked: false,
  snapping: true,
  pollSeconds: 1,
  weatherPlace: null,
  soundMixerCollapsed: false,
  design: "reference",
  language: null,
  appIcon: "widget-stack",
  languages: [],
  systemLanguage: "ru",
  hiddenDisks: [],
  hiddenMetrics: {},
  sensorPicks: {},
  version: "",
  supportRare: false,
  noGpu: false,
  unsaved: false,
};

let current: SettingsView = EMPTY;
const listeners = new Set<() => void>();
let started = false;

function publish(next: SettingsView) {
  current = next;
  for (const listener of listeners) listener();
}

function start() {
  if (started) return;
  started = true;

  // Сперва подписка, потом снимок: правка, разосланная до того, как слушатель
  // встал, иначе пропала бы, а снимок мог быть снят ещё до неё — и окно стояло
  // бы на прежних флагах до следующей рассылки (как в `live.ts`). Порядок
  // ответа и событий всё равно не гарантирован: правка, случившаяся сразу
  // после открытия окна, придёт событием, а следом опоздавший снимок откатил
  // бы её назад. То же правило, что у вида в `look.ts`
  let heard = false;

  listen<SettingsView>("settings", (event) => {
    heard = true;
    publish(event.payload);
  })
    .then(() => invoke<SettingsView>("settings_view"))
    .then((first) => {
      if (heard) return;
      publish(first);
    })
    .catch(() => {
      // Вне Tauri настроек нет — окно останется с пустым списком
    });
}

function subscribe(listener: () => void): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useSettings(): SettingsView {
  return useSyncExternalStore(subscribe, () => current);
}

/**
 * Кого показывает это окно. Пусто — окно не виджета: настройки или витрина.
 */
export const OWN_ID = new URLSearchParams(window.location.search).get("widget") ?? "";

/**
 * Своя строка в общем списке виджетов.
 *
 * Живёт здесь, а не в `view.ts`, потому что нужна не одному только виду: по
 * ней же окно узнаёт, закреплено ли оно. Двум разборам адреса разойтись было
 * бы вопросом времени, а строка — одна и та же.
 */
export function useOwnRow(): WidgetRow | undefined {
  return useSettings().widgets.find((widget) => widget.id === OWN_ID);
}

/**
 * Своё состояние тут никто не правит: ответом придёт событие, и оно же
 * разойдётся по всем, кто на настройки подписан.
 */
export function setWidgetOption(id: string, option: WidgetOption, value: boolean): void {
  void invoke("set_widget_option", { id, option, value });
}

export function setAllWidgetOption(option: WidgetOption, value: boolean): void {
  void invoke("set_all_widget_option", { option, value });
}

/**
 * Включает или выключает модуль: выключенный не грузится вовсе — ни окна, ни
 * фоновых работ. Своей командой, а не столбцом `set_widget_option`: она
 * собирает и разбирает окно, а это Rust делает только из async-команды.
 */
export function setModule(id: string, on: boolean): Promise<void> {
  return invoke<void>("set_module", { id, on });
}

export function setAllModules(on: boolean): Promise<void> {
  return invoke<void>("set_all_modules", { on });
}

/** Вид виджета: чем карточка себя рисует. Номер — место в `views`. */
export function setWidgetView(id: string, index: number): void {
  void invoke("set_widget_view", { id, index });
}

// Быстрые повторные нажатия должны записываться в том же порядке.
let mixerWrites: Promise<void> = Promise.resolve();

export function setSoundMixerCollapsed(collapsed: boolean): Promise<void> {
  const write = mixerWrites.then(() => invoke<void>("set_sound_mixer_collapsed", { collapsed }));
  mixerWrites = write.catch(() => {});
  return write;
}

/** Компоновка независимо от вида графика. */
export function setWidgetLayout(id: string, index: number): void {
  void invoke("set_widget_layout", { id, index });
}

/** Готовый размер окна. Номер — место в `sizes`. */
export function setWidgetSize(id: string, index: number): void {
  void invoke("set_widget_size", { id, index });
}

/** Чем в карточке служит обложка. Номер — место в `covers`. */
export function setWidgetCover(id: string, index: number): void {
  void invoke("set_widget_cover", { id, index });
}

export function setFlag(name: Flag, value: boolean): void {
  void invoke("set_flag", { name, value });
}

/** Выбрать и запомнить значок в системном трее. */
export function setAppIcon(id: string): Promise<void> {
  return invoke("set_app_icon", { id });
}

export function setSource(seconds: number): void {
  void invoke("set_source", { seconds });
}

/** Город виджета погоды. `null` — убрать: тогда виджет никуда не ходит. */
export function setWeatherPlace(place: Place | null): void {
  void invoke("set_weather_place", { place });
}

/** Спрятать накопитель в карточке или вернуть его. */
export function setDiskHidden(name: string, hidden: boolean): void {
  void invoke("set_disk_hidden", { name, hidden });
}

/** Снять ряд с карточки процессора или видеокарты либо вернуть его. */
export function setMetricHidden(widget: string, metric: Metric, hidden: boolean): void {
  void invoke("set_metric_hidden", { widget, metric, hidden });
}

/**
 * Выбрать датчик платы руками: `null` — автоматика, `off` — не показывать,
 * иначе путь датчика без имени машины.
 */
export function setSensorPick(slot: SensorSlot, pick: string | null): void {
  void invoke("set_sensor_pick", { slot, pick });
}

/**
 * Ищет город по названию у геокодера Open-Meteo.
 *
 * Единственная команда настроек, которая что-то отдаёт: находки нигде не
 * хранятся, событием им приезжать неоткуда.
 */
export function searchPlaces(query: string): Promise<Place[]> {
  return invoke<Place[]>("weather_search", { query });
}

/**
 * Открывает настройки одного виджета — страницу в общем окне настроек.
 * Открытое окно переводится на нового виджета, а не открывается вторым.
 *
 * `page` — страница внутри виджета: у почты это ящик по ключу или `new`.
 */
export function openWidgetSettings(id: string, page?: string): void {
  void invoke("open_widget_settings", { id: page === undefined ? id : `${id}/${page}` });
}

/** Расставляет открытые окна заново — спасение улетевшего за край экрана. */
export function resetPlaces(): void {
  void invoke("reset_widget_places");
}

/** Открывает папку с `settings.json`, учётными данными и историей замеров. */
export function openConfigDir(): void {
  void invoke("open_config_dir");
}

/** «Напоминать реже» — то же поле, что у кнопки карточки напоминания. */
export function setSupportRare(rare: boolean): void {
  void invoke("set_support_reminder_rare", { rare });
}

/** Папка журнала ошибок (`journal.rs`): его прикладывают к жалобе. */
export function openLogDir(): void {
  void invoke("open_log_dir");
}

/**
 * Закрывает окно настроек. Через команду, а не `window.close()`: место окна
 * пишется на диск на стороне Rust, и закрытое из JS окно туда бы не попало.
 *
 * В хвост очереди вида, а не мимо неё: `close_settings` запоминает накрученное
 * ползунками, и обгони он последние `preview` — на диск ушло бы число на шаг
 * раньше, а окна остались бы не пересчитаны под последнее.
 */
export function closeSettings(): void {
  void lookSettled().then(() => invoke("close_settings"));
}
