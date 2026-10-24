import { lazy, type ComponentType, type LazyExoticComponent } from "react";

export type WidgetDefinition = {
  id: string;
  component: LazyExoticComponent<ComponentType>;
};

/**
 * Реестр виджетов: какой компонент показывать в окне с этим `id`.
 *
 * Размер окна, рамка и признак «открыт по умолчанию» живут в
 * `src-tauri/src/widgets.rs` — окна создаёт Rust, и знать о них фронту
 * незачем. Общее у двух реестров только `id`, и его сходство проверяет
 * тест `every_widget_exists_on_the_front`. Названия живут в каталоге строк
 * (`widget.<id>` в `src/i18n`), одном на обе стороны: своя копия тут однажды
 * уже разошлась бы с переводом.
 *
 * ВАЖНО: **компонент грузится лениво, `import()` по требованию окна.** Окно
 * показывает один виджет, а статический импорт тянул в каждое окно код всех
 * двадцати — и вместе с ним подписку `lib/history.ts`, которая при загрузке
 * модуля начинает копить телеметрию. Так историю двухсот датчиков раз в
 * секунду собирали и заметки, и генератор паролей. Теперь её копят только
 * окна, которые её рисуют.
 *
 * Строку `import("./<папка>/…")` читает тест `every_telemetry_reader_needs_the_sidecar`
 * в `modules.rs`: по ней он находит папку виджета.
 */
export const WIDGETS: Record<string, WidgetDefinition> = {
  gpu: {
    id: "gpu",
    component: lazy(() => import("./hardware/HardwareWidget").then((module) => ({ default: module.GpuWidget }))),
  },
  cpu: {
    id: "cpu",
    component: lazy(() => import("./hardware/HardwareWidget").then((module) => ({ default: module.CpuWidget }))),
  },
  battery: {
    id: "battery",
    component: lazy(() => import("./battery/BatteryWidget").then((module) => ({ default: module.BatteryWidget }))),
  },
  processes: {
    id: "processes",
    component: lazy(() => import("./processes/ProcessesWidget").then((module) => ({ default: module.ProcessesWidget }))),
  },
  disks: {
    id: "disks",
    component: lazy(() => import("./hardware/HardwareWidget").then((module) => ({ default: module.DisksWidget }))),
  },
  network: {
    id: "network",
    component: lazy(() => import("./network/NetworkWidget").then((module) => ({ default: module.NetworkWidget }))),
  },
  fps: {
    id: "fps",
    component: lazy(() => import("./fps/FpsWidget").then((module) => ({ default: module.FpsWidget }))),
  },
  weather: {
    id: "weather",
    component: lazy(() => import("./weather/WeatherWidget").then((module) => ({ default: module.WeatherWidget }))),
  },
  speedtest: {
    id: "speedtest",
    component: lazy(() => import("./speedtest/SpeedtestWidget").then((module) => ({ default: module.SpeedtestWidget }))),
  },
  sound: {
    id: "sound",
    component: lazy(() => import("./sound/SoundWidget").then((module) => ({ default: module.SoundWidget }))),
  },
  clipboard: {
    id: "clipboard",
    component: lazy(() => import("./clipboard/ClipboardWidget").then((module) => ({ default: module.ClipboardWidget }))),
  },
  notes: {
    id: "notes",
    component: lazy(() => import("./notes/NotesWidget").then((module) => ({ default: module.NotesWidget }))),
  },
  todo: {
    id: "todo",
    component: lazy(() => import("./todo/TodoWidget").then((module) => ({ default: module.TodoWidget }))),
  },
  mail: {
    id: "mail",
    component: lazy(() => import("./mail/MailWidget").then((module) => ({ default: module.MailWidget }))),
  },
  password: {
    id: "password",
    component: lazy(() => import("./password/PasswordWidget").then((module) => ({ default: module.PasswordWidget }))),
  },
  totp: {
    id: "totp",
    component: lazy(() => import("./totp/TotpWidget").then((module) => ({ default: module.TotpWidget }))),
  },
  spotify: {
    id: "spotify",
    component: lazy(() => import("./spotify/SpotifyWidget").then((module) => ({ default: module.SpotifyWidget }))),
  },
  media: {
    id: "media",
    component: lazy(() => import("./media/MediaWidget").then((module) => ({ default: module.MediaWidget }))),
  },
  converter: {
    id: "converter",
    component: lazy(() => import("./converter/ConverterWidget").then((module) => ({ default: module.ConverterWidget }))),
  },
  downloader: {
    id: "downloader",
    component: lazy(() => import("./downloader/DownloaderWidget").then((module) => ({ default: module.DownloaderWidget }))),
  },
  debug: {
    id: "debug",
    component: lazy(() => import("./debug/DebugWidget").then((module) => ({ default: module.DebugWidget }))),
  },
  kitchensink: {
    id: "kitchensink",
    component: lazy(() => import("./kitchensink/KitchenSinkWidget").then((module) => ({ default: module.KitchenSinkWidget }))),
  },
  /*
   * Не виджет: окно настроек открывают из трея, а настройки одного виджета —
   * страница внутри него же. Создаёт его `src-tauri/src/settings_window.rs`, в
   * реестре Rust его нет — иначе оно получило бы строку в собственном списке
   * и открывалось бы при старте.
   */
  settings: {
    id: "settings",
    component: lazy(() => import("./settings/SettingsWindow").then((module) => ({ default: module.SettingsWindow }))),
  },
  /*
   * Тоже не виджет: карточка напоминания о донатах. Собирает и разбирает её
   * `src-tauri/src/support_reminder.rs` — только на время показа.
   */
  "support-reminder": {
    id: "support-reminder",
    component: lazy(() => import("./support/ReminderCard").then((module) => ({ default: module.ReminderCard }))),
  },
};

export function resolveWidget(id: string): WidgetDefinition | undefined {
  return WIDGETS[id];
}
