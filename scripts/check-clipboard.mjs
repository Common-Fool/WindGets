import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { transformWithOxc } from 'vite';
import * as runtime from 'react/jsx-runtime';

// Настоящие обработчики/IPC-обёртка с демонстрационными строками.
// Системный буфер, пользовательские данные и живые окна не используются.
const root = fileURLToPath(new URL('../', import.meta.url));
const states = [], refs = [], effects = [], calls = [];
let stateAt = 0, refAt = 0, effectAt = 0;
const react = {
  useState(initial) {
    const at = stateAt++;
    if (!(at in states)) states[at] = initial;
    return [states[at], value => { states[at] = typeof value === 'function' ? value(states[at]) : value; }];
  },
  useRef(initial) { const at = refAt++; return refs[at] ??= { current: initial }; },
  useEffect(effect) { const at = effectAt++; if (!(at in effects)) effects[at] = effect(); },
  useSyncExternalStore(_subscribe, snapshot) { return snapshot(); },
};
let reply = () => Promise.reject('clearFailed');
async function load(relative, imports, extra = '') {
  const filename = path.join(root, relative);
  const source = fs.readFileSync(filename, 'utf8');
  const transformed = await transformWithOxc(source, filename, { jsx: { runtime: 'automatic' } });
  const names = [...source.matchAll(/^export (?:async )?(?:const|function) (\w+)/gm)].map(match => match[1]);
  const code = transformed.code
    .replace(/^import\s+\{([^}]+)\}\s+from\s+["']([^"']+)["'];?/gm, (_, names, name) =>
      `const {${names.replace(/\s+as\s+/g, ': ')}} = require(${JSON.stringify(name)});`)
    .replace(/^import ["'][^"']+\.css["'];?/gm, '')
    .replace(/^export /gm, '') + `\nmodule.exports = {${names.join(',')}};`;
  const module = { exports: {} };
  vm.runInNewContext(code + extra, {
    module,
    require(name) {
      if (name === 'react/jsx-runtime') return runtime;
      if (name === 'react') return react;
      if (Object.hasOwn(imports, name)) return imports[name];
      throw new Error('Unexpected import: ' + name);
    },
    console: { warn() {} },
  }, { filename });
  return module.exports;
}
const controls = await load('src/lib/clipboard.ts', {
  '@tauri-apps/api/core': { invoke: async (name, args) => {
    assert.equal(name, 'clipboard_control');
    calls.push(args.action);
    return reply(args.action);
  } },
  '@tauri-apps/api/event': { listen: async () => () => {} },
});
const entry = { id: 1, preview: 'demo', formattedPreview: 'demo', chars: 4, lines: 1, current: true };
const demo = { entries: [entry], capacity: 20 };
const widget = await load('src/widgets/clipboard/ClipboardWidget.tsx', {
  '../../lib/drag': { useWindowDrag() {} },
  '../../lib/i18n': { useText: () => key => key },
  '../../lib/clipboard': { ...controls, useClipboard: () => demo },
  '../../lib/look': { useLook: () => ({ effectiveDesign: 'reference' }) },
  '../../components/Icons': { CloseIcon() {}, EraseIcon() {}, FailedIcon() {} },
  '../../components/GlassCard': { GlassCard() {} },
  '../../components/Button': { Button() {} },
}, '\nmodule.exports.ReferenceClipboard = ReferenceClipboard;');
function render() { stateAt = 0; refAt = 0; effectAt = 0; return widget.ReferenceClipboard(); }
function find(element, name) {
  if (!element || typeof element !== 'object') return undefined;
  if (element.props?.className === name || element.type?.name === name) return element;
  const children = element.props?.children;
  for (const child of Array.isArray(children) ? children.flat(Infinity) : [children]) {
    const found = find(child, name);
    if (found) return found;
  }
}
const tick = () => new Promise(resolve => setImmediate(resolve));
const button = () => find(render(), 'clip-ref__clear');
const error = () => find(render(), 'clip-ref__error');

button().props.onClick();
assert.equal(button().props.disabled, true, 'Очистка ждёт подтверждение');
assert.equal(button().props['aria-busy'], true);
demo.entries = []; // Rust забывает историю даже при отказе ОС.
await tick();
assert.equal(error().props.title, 'clipboard.clearFailed', 'Отказ очистки отличается от копирования');
assert.equal(button().props.disabled, false, 'Пустая история не запрещает повтор очистки ОС');

reply = () => Promise.resolve();
button().props.onClick();
await tick();
assert.equal(error(), undefined, 'Успех убирает отказ очистки');
assert.equal(button().props.disabled, true, 'После успеха пустая история больше не требует очистки');

demo.entries = [entry];
reply = () => Promise.reject('copyFailed');
find(render(), 'ReferenceRow').props.onCopy();
await tick();
assert.equal(error().props.title, 'clipboard.copyFailed');
assert.equal(find(render(), 'ReferenceRow').props.failed, true);

// Запоздалый отказ очистки не перекрывает результат нового действия.
let failOld;
reply = () => new Promise((_, reject) => { failOld = reject; });
button().props.onClick();
await tick();
reply = () => Promise.resolve();
find(render(), 'ReferenceRow').props.onCopy();
await tick();
failOld('clearFailed');
await tick();
assert.equal(error(), undefined);
assert.equal(button().props['aria-busy'], false);

// Разобранный дизайн не применяет свой старый ACK.
reply = () => new Promise((_, reject) => { failOld = reject; });
button().props.onClick();
await tick();
const beforeUnmount = [...states];
effects.forEach(cleanup => cleanup?.());
failOld('clearFailed');
await tick();
assert.deepEqual(states, beforeUnmount);
assert.deepEqual(calls.map(action => action.kind), ['clear', 'clear', 'copy', 'clear', 'copy', 'clear']);
console.log('Clipboard regressions passed: clear refusal/retry, pending, success, copy refusal, stale ACK and unmount.');
