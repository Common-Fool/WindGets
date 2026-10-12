import assert from 'node:assert/strict';
import { fitWave } from '../src/widgets/converter/wave.ts';

for (const scale of [0.9, 1, 1.3]) {
  for (const width of [1, 24, 70, 110, 330]) {
    const silence = fitWave(Array(120).fill(0), width, scale);
    assert.ok(silence.length >= 1 && silence.length <= 120);
    assert.ok(silence.every((value) => value === 0));
    assert.ok(silence.length * 2 * scale + Math.max(0, silence.length - 1) <= Math.max(width, 2 * scale));
    // A single loud sample at either edge or the middle must survive narrowing.
    for (const index of [0, 59, 119]) {
      const samples = Array(120).fill(0); samples[index] = 1;
      const bars = fitWave(samples, width, scale);
      assert.equal(Math.max(...bars), 1);
      assert.equal(samples[index], 1);
    }
  }
}
assert.deepEqual(fitWave([], 70, 1), []);
assert.deepEqual(fitWave([0.2, 0.7, 0.1, 1], 5, 1), [0.7, 1]);
assert.deepEqual(fitWave([0.1, 0.9], 330, 1), [0.1, 0.9]);
console.log('Wave regression passed: narrow tracks, peaks, silence, source preservation and 90/100/130% scale.');
