import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// Перетаскивание окна включает одна оболочка (`Shell` в `main.tsx`) на все
// окна. Свой вызов в виджете дал бы два перетаскивания на одно нажатие, а
// пропавший в оболочке — окна, которые не двигаются (так было с зарядом)
const root = fileURLToPath(new URL('../src/', import.meta.url));
const call = /\buseWindowDrag\s*\(/;

function sources(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path);
    return /\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

const callers = sources(root)
  .filter((path) => readFileSync(path, 'utf8').split('\n').some((line) => !/^\s*(\/\/|\*|\/\*|\{\/\*)/.test(line) && call.test(line) && !/export function useWindowDrag/.test(line)))
  .map((path) => relative(root, path).split(sep).join('/'));

assert.deepEqual(callers, ['main.tsx'], `useWindowDrag зовёт только Shell, а зовут: ${callers.join(', ')}`);
console.log('check-window-drag: перетаскивание включает одна оболочка');
