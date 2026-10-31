import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';

// Только геометрия на вымышленных размерах: пользовательские записи не нужны.
const source = readFileSync(new URL('../src/widgets/totp/fit.ts', import.meta.url), 'utf8');
const code = stripTypeScriptTypes(source)
  .replace(/^import .*;\r?$/gm, '')
  .replace(/^export /gm, '') + '\nmodule.exports = { fitTotpRows };';
const context = vm.createContext({ module: { exports: {} } });
new vm.Script(code).runInContext(context);
const { fitTotpRows: fit } = context.module.exports;

for (const pitch of [30, 36, 40, 43.59375, 48, 52, 62.4]) {
  let state = fit(8 * pitch, pitch, null);
  assert.equal(state.rows, 8);
  state = fit(7.7001 * pitch, pitch, state);
  assert.equal(state.rows, 8, 'Меньше 30 % перекрытия ещё не скрывают строку');
  state = fit(7.7 * pitch, pitch, state);
  assert.equal(state.rows, 7, 'Ровно 30 % перекрытия скрывают строку');
  state = fit(7 * pitch, pitch, state);
  state = fit(7.4999 * pitch, pitch, state);
  assert.equal(state.rows, 7, 'Меньше 50 % открытия ещё не показывают строку');
  state = fit(7.5 * pitch, pitch, state);
  assert.equal(state.rows, 8, 'Ровно 50 % открытия показывают строку');
  for (let render = 0; render < 10; render++) {
    assert.equal(fit(state.room, state.pitch, state), state, 'Отрисовка без нового размера не меняет видимость');
  }
  state = fit(7.6 * pitch, pitch, state);
  assert.equal(state.rows, 8);
  state = fit(7.59 * pitch, pitch, state);
  assert.equal(state.rows, 7, 'При смене направления действует порог уменьшения');
  state = fit(7.6 * pitch, pitch, state);
  assert.equal(state.rows, 8, 'При смене направления действует порог увеличения');

  state = fit(2.7 * pitch, pitch, state);
  assert.equal(state.rows, 2, 'Резкое уменьшение убирает несколько строк');
  state = fit(9.5 * pitch, pitch, state);
  assert.equal(state.rows, 10, 'Резкое увеличение добавляет несколько строк');
  state = fit(19 * pitch, pitch * 2, state);
  assert.equal(state.rows, 10, 'Пропорциональный масштаб сохраняет видимость');
}

assert.equal(fit(0, 40, null), null);
assert.equal(fit(100, 0, null), null);
assert.equal(fit(NaN, 40, null), null);
assert.equal(fit(100, Infinity, null), null);
assert.equal(fit(19.99, 40, null).rows, 0);
assert.equal(fit(20, 40, null).rows, 1);
assert.equal(fit(28, 40, fit(40, 40, null)).rows, 0);
console.log('TOTP row regressions passed: 30% covered / 50% revealed, repeated renders, resize reversals, jumps and scale.');
