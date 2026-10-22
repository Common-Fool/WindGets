import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

/**
 * Живое превью виджета в окне настроек (`src/preview/install.ts`) отвечает на
 * вызовы IPC демоданными. Подменить сам `window.__TAURI_INTERNALS__` в рамке
 * нельзя: wry на Windows внедряет скрипты Tauri во все фреймы, а Tauri
 * объявляет объект через `defineProperty` — ни перезаписать, ни поправить.
 *
 * Поэтому подмена — в модулях `@tauri-apps/api`: каждое чтение внутренностей
 * Tauri сперва смотрит на подделку превью. В обычных окнах её нет, и чтение
 * доходит до настоящего объекта, как прежде. В API есть только чтения этих
 * объектов, присваиваний нет — их замена скобками сломала бы синтаксис.
 */
function previewIpc(): Plugin {
  return {
    name: "windgets-preview-ipc",
    enforce: "pre",
    transform(code, id) {
      // Путь у Vite всегда с прямыми косыми; в разработке бывает хвост `?v=…`
      if (!/\/@tauri-apps\/api\/(?!mocks\.)[^/?]+\.js(\?|$)/.test(id)) return null;
      return code
        .replaceAll("window.__TAURI_INTERNALS__", "(window.__WINDGETS_PREVIEW_IPC__ ?? window.__TAURI_INTERNALS__)")
        .replaceAll(
          "window.__TAURI_EVENT_PLUGIN_INTERNALS__",
          "(window.__WINDGETS_PREVIEW_EVENTS__ ?? window.__TAURI_EVENT_PLUGIN_INTERNALS__)",
        );
    },
  };
}

export default defineConfig({
  plugins: [previewIpc(), react()],
  // В разработке API отдаётся как есть, без предсборки esbuild: иначе плагин
  // выше его не увидел бы
  optimizeDeps: { exclude: ["@tauri-apps/api"] },
  // Tauri сам пишет в консоль, чистка экрана Vite мешает читать его вывод
  clearScreen: false,
  server: {
    // Порт фиксированный: он же прописан в tauri.conf.json как devUrl
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    // Пересборка фронта не должна дёргаться на изменения в Rust
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WebView2 на этой машине свежий, но занижать цель безопаснее
    target: "chrome110",
    sourcemap: false,
  },
});
