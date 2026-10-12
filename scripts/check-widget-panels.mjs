import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';

// Настоящие хуки, подставной IPC: без запуска окон и чтения пользовательских данных.
function load(relative, exports, modules) {
  const source = readFileSync(new URL(relative, import.meta.url), 'utf8');
  const code = stripTypeScriptTypes(source)
    .replace(/^import\s+\{([^}]+)\}\s+from\s+"([^"]+)";/gm,
      (_, names, path) => `const {${names}} = require(${JSON.stringify(path)});`)
    .replace(/^export /gm, '') + `\nmodule.exports = {${exports.join(',')}};`;
  const context = vm.createContext({ module: { exports: {} }, require: (name) => {
    assert.ok(modules[name], `Подставлен импорт ${name}`);
    return modules[name];
  } });
  new vm.Script(code, { filename: relative }).runInContext(context);
  return context.module.exports;
}

const slots = [];
let cursor = 0;
let pendingEffects = [];
const same = (a, b) => a && b && a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
const react = {
  useState(initial) {
    const at = cursor++;
    if (!(at in slots)) slots[at] = initial;
    return [slots[at], (value) => { slots[at] = typeof value === 'function' ? value(slots[at]) : value; }];
  },
  useRef(initial) { const at = cursor++; return slots[at] ??= { current: initial }; },
  useMemo(callback, deps) {
    const at = cursor++;
    if (!same(slots[at]?.deps, deps)) slots[at] = { deps, value: callback() };
    return slots[at].value;
  },
  useCallback(callback, deps) { return react.useMemo(() => callback, deps); },
  useEffect(callback, deps) {
    const at = cursor++;
    if (!same(slots[at]?.deps, deps)) pendingEffects.push(() => {
      slots[at]?.cleanup?.();
      slots[at] = { deps, cleanup: callback() };
    });
  },
};
const calls = [];
const look = { scale: 1, effectiveDesign: 'app' };
let sizeHandler;
let resetHandler;
const api = load('../src/lib/grow.ts', ['usePresetWindowGrowth', 'useWindowGrowth'], {
  react,
  '@tauri-apps/api/core': { invoke: async (command, args) => calls.push({ command, args }) },
  './look': { useLook: () => look },
  './resized': { useWindowResized: (handler) => { sizeHandler = handler; },
    useWindowReset: (handler) => { resetHandler = handler; } },
});
function render(view = 0, size = 0, design = 'reference', layout = 0) {
  cursor = 0;
  pendingEffects = [];
  const result = api.usePresetWindowGrowth(view, size, design, layout);
  pendingEffects.forEach((effect) => effect());
  return result;
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
let growth = render();
growth.open(188);
growth = render(0, 0, 'app', 0);
await tick();
assert.equal(calls.length, 0, 'Старая очередь GPT не правит уже выбранный Claude');
growth.open(244);
await tick();
assert.equal(calls.at(-1).args.preset.design, 'app');
growth.open(244);
growth = render(0, 0, 'app', 1);
growth.open(244);
await tick();
assert.equal(calls.length, 2, 'Предыдущая компоновка отменена, новая применена один раз');
assert.equal(calls.at(-1).args.preset.layout, 1);
growth.open(244);
look.scale = 1.3;
growth = render(0, 0, 'app', 1);
growth.open(244 * 1.3);
await tick();
assert.equal(calls.length, 3, 'Прибавка старого масштаба отменена');
assert.equal(calls.at(-1).args.preset.scale, 1.3);
growth.open(244);
sizeHandler();
growth.close();
await tick();
assert.equal(calls.length, 4, 'Событие размера отменило предыдущую прибавку');
assert.equal(calls.at(-1).args.preset.extra, 0);
// Сброс основы оставляет панель открытой и требует повторного измерения.
growth.open(188);
const beforeReset = growth;
resetHandler();
growth = render(0, 0, 'app', 1);
assert.notEqual(growth, beforeReset, 'Сброс повторно запускает эффект открытой панели');
growth.open(180 * 1.3);
await tick();
assert.equal(calls.length, 5, 'Рост, стоявший в очереди до сброса, отменён');
assert.equal(calls.at(-1).args.preset.extra, 180 * 1.3);
growth.close();
await tick();
assert.equal(calls.at(-1).args.preset.extra, 0, 'Закрытие возвращает новую основу');
growth.open(188);
slots.forEach((slot) => slot?.cleanup?.());
await tick();
assert.equal(calls.length, 6, 'Разобранное окно больше не меняет геометрию');

// Долг истории Claude не вычитается после смены дизайна загрузчика.
slots.length = 0;
look.scale = 1;
const legacyStart = calls.length;
function renderLegacy() {
  cursor = 0;
  pendingEffects = [];
  const result = api.useWindowGrowth();
  pendingEffects.forEach((effect) => effect());
  return result;
}
let historyGrowth = renderLegacy();
historyGrowth.open(120);
look.effectiveDesign = 'reference';
historyGrowth = renderLegacy();
historyGrowth.close();
await tick();
assert.equal(calls.length, legacyStart, 'Старая прибавка и её закрытие не меняют новую основу GPT');
look.effectiveDesign = 'app';
historyGrowth = renderLegacy();
historyGrowth.open(80);
await tick();
assert.equal(calls.at(-1).args.by, 80, 'Claude измеряет открытую историю от своей основы');
historyGrowth.close();
await tick();
assert.equal(calls.at(-1).args.by, -80, 'Закрытие возвращает точную основу');
historyGrowth.open(110);
sizeHandler();
historyGrowth.close();
await tick();
assert.equal(calls.length, legacyStart + 2, 'Готовый размер отменяет долг и отложенный рост');
historyGrowth.open(100);
const historyBeforeReset = historyGrowth;
resetHandler();
historyGrowth = renderLegacy();
assert.notEqual(historyGrowth, historyBeforeReset, 'История повторно измеряется после сброса');
historyGrowth.open(140);
await tick();
assert.equal(calls.length, legacyStart + 3, 'Старый рост отменён, новый применяется без вычитания долга');
assert.equal(calls.at(-1).args.by, 140);
historyGrowth.close();
await tick();
assert.equal(calls.at(-1).args.by, -140);
historyGrowth.open(100);
slots.forEach((slot) => slot?.cleanup?.());
await tick();
assert.equal(calls.length, legacyStart + 4, 'История разобранного окна не меняет высоту');

// Маршрутизация событий размера — обе стороны: отправитель в Rust и слушатель
// во фронте. Доставка повторяет Tauri 2 (match_any_or_filter в event/listener.rs):
// глобальный emit без фильтра достаётся КАЖДОМУ слушателю, с любым адресом;
// emit_to(label) — только слушателям этого окна и слушателям без адреса.
// Подменять первое вторым нельзя: прежний тест так и делал и пропускал ошибку
// отправителя.
const rust = readFileSync(new URL('../src-tauri/src/windows.rs', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const EVENT_OF = { SIZE_EVENT: 'widget-size', RESET_EVENT: 'widget-reset' };
// Как отправляет Rust: по каждому месту отправки — событие и вид рассылки.
// tell(window, EV) и window.emit_to(.., EV) — адресно, window.emit(EV) — всем.
const EV = '(SIZE_EVENT|RESET_EVENT|"widget-size"|"widget-reset")';
const SEND = new RegExp(`tell\\(\\s*&?window\\s*,\\s*${EV}|window\\.emit_to\\([^,]*,\\s*${EV}|window\\.emit\\(\\s*${EV}`, 'g');
function sendsIn(fn) {
  const at = rust.indexOf('pub fn ' + fn + '(');
  assert.ok(at >= 0, fn + ' есть в windows.rs');
  const body = rust.slice(at, rust.indexOf('\n}\n', at));
  return [...body.matchAll(SEND)].map(([, viaTell, viaTo, viaAll]) => {
    const event = viaTell ?? viaTo ?? viaAll;
    return { kind: viaAll ? 'all' : 'to', event: EVENT_OF[event] ?? event.slice(1, -1) };
  });
}
const setSizeSends = sendsIn('set_size');
assert.deepEqual(setSizeSends, [{ kind: 'to', event: 'widget-size' }], 'set_size говорит о размере только своему окну');
const resetSends = sendsIn('reset_places');
assert.deepEqual(resetSends, [{ kind: 'to', event: 'widget-size' }, { kind: 'to', event: 'widget-reset' }],
  'Сброс: звуку widget-size, остальным widget-reset — оба адресно');
assert.match(rust, /if widget\.id == "sound" \{[^}]*tell\(window, SIZE_EVENT/, 'widget-size при сбросе — у звука');
assert.match(rust, /\} else \{[^}]*tell\(window, RESET_EVENT/, 'widget-reset при сбросе — у остальных');
assert.match(rust, /const SIZE_EVENT: &str = "widget-size";/);
assert.match(rust, /const RESET_EVENT: &str = "widget-reset";/);
assert.match(rust, /fn tell<[^>]*>\(window: &WebviewWindow, event: &str, payload: S\) \{\s*let _ = window\.emit_to\(window\.label\(\), event, payload\);/,
  'tell шлёт событие только своему окну');
assert.doesNotMatch(rust, /\.emit\(\s*("widget-(size|reset)"|SIZE_EVENT|RESET_EVENT)/,
  'Ни widget-size, ни widget-reset не уходят общей рассылкой нигде в windows.rs');

const listeners = [];
function deliver(kind, label, event) {
  listeners.filter((item) => item.event === event
    && (kind === 'all' || item.target === null || item.target === label))
    .forEach((item) => item.handler());
}
function tauriWindow(label) {
  return { listen: (event, handler) => {
    const entry = { target: label, event, handler };
    listeners.push(entry);
    return Promise.resolve(() => { listeners.splice(listeners.indexOf(entry), 1); });
  } };
}
// Карточка с панелью: widget-size закрывает её (useWindowResized), widget-reset
// оставляет открытой и заново меряет высоту (useWindowReset).
function mountCard(label) {
  const card = { label, open: true, closes: 0, remeasures: 0, cleanups: [] };
  const hooks = load('../src/lib/resized.ts', ['useWindowResized', 'useWindowReset'], {
    react: { useRef: (current) => ({ current }), useEffect: (effect) => { card.cleanups.push(effect()); } },
    '@tauri-apps/api/webviewWindow': { getCurrentWebviewWindow: () => tauriWindow(label) },
  });
  hooks.useWindowResized(() => { card.open = false; card.closes += 1; });
  hooks.useWindowReset(() => { card.remeasures += 1; });
  return card;
}
const spotify = mountCard('spotify');
const weather = mountCard('weather');
const sound = mountCard('sound');
await tick();
// Контроль модели: настоящая общая рассылка закрыла бы чужую панель.
deliver('all', 'weather', 'widget-size');
assert.equal(spotify.open, false, 'Модель доставки честна: глобальный emit достаётся чужому окну');
for (const card of [spotify, weather, sound]) { card.open = true; card.closes = 0; }
// Два окна: у Spotify раскрыты плейлисты, погоде меняют размер так, как шлёт set_size.
for (const send of setSizeSends) deliver(send.kind, 'weather', send.event);
assert.equal(weather.closes, 1, 'Окно, которому сменили размер, закрывает свою панель');
assert.equal(spotify.open, true, 'Раскрытые плейлисты соседнего окна остаются');
assert.equal(spotify.closes + spotify.remeasures, 0, 'Геометрию соседа событие не трогает вовсе');
weather.open = true; weather.closes = 0;
// Сброс раскладки — с теми событиями и тем способом отправки, что у reset_places.
for (const card of [spotify, weather, sound]) {
  const send = card.label === 'sound' ? resetSends[0] : resetSends[1];
  deliver(send.kind, card.label, send.event);
}
assert.equal(sound.open, false, 'Сброс закрывает панели звука');
assert.equal(sound.remeasures, 0);
for (const card of [spotify, weather]) {
  assert.equal(card.open, true, card.label + ': раскрытие переживает сброс');
  assert.equal(card.remeasures, 1, card.label + ': высота панели меряется заново один раз');
  assert.equal(card.closes, 0);
}
// Разобранное окно больше не слушает.
spotify.cleanups.forEach((cleanup) => cleanup?.());
await tick();
deliver('to', 'spotify', 'widget-reset');
assert.equal(spotify.remeasures, 1, 'После отписки события до карточки не доходят');

// StrictMode: эффект, уборка, эффект — подписка ровно одна.
let strictCount = 0;
const strictHooks = load('../src/lib/resized.ts', ['useWindowResized'], {
  react: { useRef: (current) => ({ current }), useEffect: (effect) => {
    const first = effect(); first?.(); effect();
  } },
  '@tauri-apps/api/webviewWindow': { getCurrentWebviewWindow: () => tauriWindow('notes') },
});
strictHooks.useWindowResized(() => { strictCount += 1; });
await tick();
deliver('to', 'notes', 'widget-size');
assert.equal(strictCount, 1, 'Повторное монтирование не оставляет второй подписки');

// Вне Tauri: метаданных окна нет, getCurrentWebviewWindow бросает TypeError;
// вторая проверка — подписка есть, но асинхронно отказывает.
const unhandled = [];
const onUnhandled = (reason) => unhandled.push(reason);
process.on('unhandledRejection', onUnhandled);
for (const getCurrentWebviewWindow of [
  () => { throw new TypeError("Cannot read properties of undefined (reading 'metadata')"); },
  () => ({ listen: () => Promise.reject(new Error('IPC недоступен')) }),
]) {
  const cleanups = [];
  const browser = load('../src/lib/resized.ts', ['useWindowResized', 'useWindowReset'], {
    react: { useRef: (current) => ({ current }), useEffect: (effect) => { cleanups.push(effect()); } },
    '@tauri-apps/api/webviewWindow': { getCurrentWebviewWindow },
  });
  assert.doesNotThrow(() => { browser.useWindowResized(() => {}); browser.useWindowReset(() => {}); },
    'Без Tauri хук пропускает подписку, а не роняет карточку');
  // Окно живёт долго: отказ обязан быть пойман ещё до уборки, а не ею.
  await tick(); await tick();
  assert.equal(unhandled.length, 0, 'Отказ асинхронной подписки перехвачен сразу, до уборки');
  assert.doesNotThrow(() => cleanups.forEach((cleanup) => cleanup?.()), 'Уборка без подписки тоже спокойна');
  await tick();
}
process.off('unhandledRejection', onUnhandled);
assert.equal(unhandled.length, 0, 'Уборка после отказа ничего не роняет');
console.log('Panel regressions passed: design/layout/scale queues, explicit size, reset with open panels, unmount, addressed size/reset routing, two-window panels, browser without Tauri.');
