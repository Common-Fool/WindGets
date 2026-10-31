import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';

// Проверяет настоящий store look.ts с подставным IPC, без запуска окон.
const source = readFileSync(new URL('../src/lib/look.ts', import.meta.url), 'utf8');
const names = [...source.matchAll(/^export (?:const|function) (\w+)/gm)].map((item) => item[1]);
const code = stripTypeScriptTypes(source)
  .replace(/^import\s+\{([^}]+)\}\s+from\s+"([^"]+)";/gm, (_, names, path) => `const {${names}} = require(${JSON.stringify(path)});`)
  .replace(/^export /gm, '') + `\nmodule.exports = {${names.join(',')}};`;
const tick = () => new Promise((resolve) => setImmediate(resolve));

function fixture(label, delayedInitial = false) {
  const handlers = [];
  const calls = [];
  const argumentsLog = [];
  const dataset = {};
  const ready = ['cpu', 'gpu', 'sound', 'processes', 'disks', 'network', 'speedtest', 'weather', 'media', 'spotify', 'clipboard', 'notes', 'todo', 'mail', 'password', 'totp', 'converter', 'downloader', 'settings'].includes(label);
  let backend = { design: 'app', effectiveDesign: 'app', cardOpacity: 0.94, scale: 1,
    palette: 'reference-classic', glass: 'graphite', accentColor: '#4CA5FF',
    customColors: { accent: '#4CA5FF', positive: '#40D9A3', caution: '#F0B857', negative: '#F17882' }, vivid: 1 };
  let resolveInitial;
  let fail = false;
  const initial = new Promise((resolve) => { resolveInitial = resolve; });
  if (!delayedInitial) resolveInitial({ ...backend });
  const listen = (event, handler, target) => {
    handlers.push({ handler, target });
    return Promise.resolve(() => {});
  };
  const modules = {
    react: { useCallback: (callback) => callback, useSyncExternalStore: (_, snapshot) => snapshot() },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      calls.push(command);
      argumentsLog.push({ command, args });
      if (command === 'appearance') return initial;
      if (command === 'set_design') {
        if (fail) throw new Error('IPC failure');
        backend = { ...backend, design: args.design, effectiveDesign: ready ? args.design : 'app' };
        return { ...backend };
      }
    } },
    // Семантика Tauri: target Any получает также события чужих окон.
    '@tauri-apps/api/event': { listen: (event, handler) => listen(event, handler, null) },
    '@tauri-apps/api/webviewWindow': { getCurrentWebviewWindow: () => ({
      label, listen: (event, handler) => listen(event, handler, label),
    }) },
  };
  const context = vm.createContext({ module: { exports: {} }, require: (name) => modules[name],
    URLSearchParams, window: { location: { search: `?widget=${label}` } },
    document: { documentElement: { dataset, style: { setProperty() {} } } } });
  new vm.Script(code, { filename: 'look.ts' }).runInContext(context);
  const api = context.module.exports;
  api.initLook();
  return { api, dataset, calls, argumentsLog, payload: () => ({ ...backend }),
    emit: (target, payload) => handlers.forEach(({ handler, target: listener }) => {
      if (listener === null || listener === target) handler({ payload });
    }), resolveInitial, fail: () => { fail = true; } };
}

for (const label of ['cpu', 'gpu', 'sound', 'processes', 'disks', 'network', 'speedtest', 'weather', 'media', 'spotify', 'clipboard', 'notes', 'todo', 'mail', 'password', 'totp', 'settings', 'converter', 'downloader', 'debug']) {
  const f = fixture(label);
  await tick();
  for (const design of ['reference', 'app', 'reference', 'app', 'reference']) {
    await f.api.setDesign(design);
    f.emit(label, f.payload());
    const effective = ['cpu', 'gpu', 'sound', 'processes', 'disks', 'network', 'speedtest', 'weather', 'media', 'spotify', 'clipboard', 'notes', 'todo', 'mail', 'password', 'totp', 'converter', 'downloader', 'settings'].includes(label) ? design : 'app';
    // Последний чужой appearance раньше отменял оформление своего окна.
    f.emit(label === 'settings' ? 'cpu' : 'settings', { ...f.payload(), effectiveDesign: effective === 'app' ? 'reference' : 'app' });
    assert.equal(f.api.useLook().design, design, `${label}: выбранный дизайн`);
    assert.equal(f.dataset.design, effective, `${label}: чужое окно не отменяет дизайн`);
  }
}

const late = fixture('cpu', true);
await late.api.setDesign('reference');
late.resolveInitial({ ...late.payload(), design: 'app', effectiveDesign: 'app' });
await tick();
assert.equal(late.dataset.design, 'reference', 'старый initial snapshot не отменяет ACK');

const ordered = fixture('gpu');
await tick();
await Promise.all([ordered.api.setDesign('reference'), ordered.api.setDesign('app')]);
assert.equal(ordered.dataset.design, 'app', 'быстрые нажатия выполняются по порядку');
ordered.fail();
await assert.rejects(ordered.api.setDesign('reference'), /IPC failure/);
assert.equal(ordered.dataset.design, 'app', 'ошибка не показывает ложный выбранный дизайн');
const profiles = fixture('settings');
await tick();
await profiles.api.setDesign('reference');
const captured = profiles.api.useLook().effectiveDesign;
await profiles.api.setDesign('app');
profiles.api.setPalette('reference-a', captured);
profiles.api.setGlass('cool', captured);
profiles.api.setAccentColor('#123456', captured);
profiles.api.setCustomColor('positive', '#ABCDEF', captured);
profiles.api.resetCustomColors(captured);
await tick();
await tick();
for (const call of profiles.argumentsLog.filter((call) => ['set_palette', 'set_glass', 'set_accent_color', 'set_custom_color', 'reset_custom_colors'].includes(call.command))) {
  assert.equal(call.args.design, 'reference', `${call.command}: late edit retains its original design`);
}
assert.equal(profiles.argumentsLog.filter((call) => call.args?.design === 'reference').length, 6);
console.log('Appearance regression passed: 20 windows, both designs, foreign events, late snapshot, command order, IPC failure and design-scoped colour edits.');
