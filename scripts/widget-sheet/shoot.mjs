// Снимки виджетов для README: лист всех виджетов в двух дизайнах и каждый
// виджет отдельно, на демоданных живого превью (`src/preview/`).
//
// Нужен работающий дев-сервер на 1420 (`npm run dev` или `npm run app`).
// Запуск: `node scripts/widget-sheet/shoot.mjs`. Пишет WebP в
// `assets/screenshots/`. Снимает Edge без окна через протокол DevTools —
// зависимостей нет, только Node 22 (fetch и WebSocket встроены).
//
// Перед съёмкой после правки реестра размеров — выгрузить их заново:
// `cargo test --lib widget_sheet_sizes -- --ignored` (в `src-tauri`).

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const EDGE = "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const SHEET = "http://localhost:1420/scripts/widget-sheet/index.html";
const OUT = new URL("../../assets/screenshots/", import.meta.url);
const PORT = 9333;
// Сколько ждать после загрузки листа: словарь, демо, засев истории, подгонка высот
const SETTLE_MS = 4000;

// Виджеты с превью — те же ключи, что выгрузил Rust в `sizes.json`
const sizes = JSON.parse(readFileSync(new URL("./sizes.json", import.meta.url), "utf8"));
const widgets = Object.keys(sizes.reference);

const shots = [
  { file: "overview-new.webp", query: "design=reference&columns=4" },
  { file: "overview-classic.webp", query: "design=app&columns=4" },
  ...widgets.map((id) => ({ file: `${id}.webp`, query: `design=reference&only=${id}` })),
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const profile = mkdtempSync(join(tmpdir(), "windgets-shoot-"));
const edge = spawn(EDGE, [
  "--headless=new",
  `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${profile}`,
  "--hide-scrollbars",
  "--force-device-scale-factor=2",
  "--window-size=1900,1400",
  "about:blank",
], { stdio: "ignore" });

try {
  let target;
  for (let attempt = 0; attempt < 50 && target === undefined; attempt++) {
    await sleep(200);
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      target = list.find((entry) => entry.type === "page");
    } catch {
      // Edge ещё поднимается
    }
  }
  if (target === undefined) throw new Error("Edge не поднял страницу для DevTools");

  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  let next = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id === undefined || !pending.has(message.id)) return;
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result);
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++next;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });

  // Прозрачный фон страницы: в кадр попадает только подложка листа
  await send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });

  mkdirSync(OUT, { recursive: true });
  for (const shot of shots) {
    await send("Page.navigate", { url: `${SHEET}?lang=en&${shot.query}` });
    await sleep(SETTLE_MS);
    const { result } = await send("Runtime.evaluate", {
      expression: "JSON.stringify(document.getElementById('sheet').getBoundingClientRect())",
      returnByValue: true,
    });
    const rect = JSON.parse(result.value);
    const { data } = await send("Page.captureScreenshot", {
      format: "webp",
      quality: 90,
      captureBeyondViewport: true,
      clip: { x: rect.x, y: rect.y, width: rect.width, height: rect.height, scale: 1 },
    });
    writeFileSync(new URL(shot.file, OUT), Buffer.from(data, "base64"));
    console.log(`${shot.file}: ${Math.round(rect.width)}×${Math.round(rect.height)}`);
  }

  socket.close();
} finally {
  edge.kill();
  // Дочерние процессы Edge отпускают профиль не сразу
  rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
