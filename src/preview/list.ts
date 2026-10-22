/**
 * Виджеты с живым превью в разделе «Виджеты». Служебные окна (отладка,
 * витрина) получают там только описание.
 *
 * Отдельным крошечным модулем, а не рядом с демоданными: список нужен окну
 * настроек, а демо — только рамке превью. Строки списка читает тест
 * `every_widget_has_a_preview` в `widgets.rs`: новый виджет обязан получить
 * демоданные (`src/preview/demo/`) и строку здесь.
 */
export const PREVIEWED: readonly string[] = [
  "gpu",
  "cpu",
  "processes",
  "disks",
  "network",
  "weather",
  "speedtest",
  "sound",
  "clipboard",
  "notes",
  "todo",
  "mail",
  "password",
  "totp",
  "spotify",
  "media",
  "converter",
  "downloader",
];
