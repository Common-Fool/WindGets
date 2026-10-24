import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

// Живое превью виджетов уводит вызовы `@tauri-apps/api` на подделку плагином
// `previewIpc` в vite.config.ts. В разработке он видит API, только если тот не
// предсобран: имя пакета подпути не исключает, и забытый подпуть молча
// возвращал непереписанный API — рамка превью оставалась пустой (22 октября).
// Поэтому каждый подпуть, который импортирует `src/`, обязан стоять в
// `optimizeDeps.exclude`.

const root = new URL('../', import.meta.url);
const config = readFileSync(new URL('vite.config.ts', root), 'utf8');

function sources(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

const imported = new Set();
for (const file of sources(new URL('src/', root).pathname.replace(/^\/([A-Za-z]:)/, '$1'))) {
  for (const match of readFileSync(file, 'utf8').matchAll(/from\s+["'](@tauri-apps\/api(?:\/[\w-]+)?)["']/g)) {
    imported.add(match[1]);
  }
}

assert.ok(imported.size > 0, 'импорты @tauri-apps/api не найдены — проверка ничего не проверяет');
assert.match(config, /name:\s*"windgets-preview-ipc"/, 'плагин previewIpc пропал из vite.config.ts');

const excluded = config.slice(config.indexOf('optimizeDeps'));
for (const path of imported) {
  assert.ok(excluded.includes(`"${path}"`), `${path} не исключён из предсборки (optimizeDeps.exclude в vite.config.ts)`);
}

console.log(`ok: ${imported.size} подпутей @tauri-apps/api исключены из предсборки`);
