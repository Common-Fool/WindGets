/** Ширина столбика остаётся положительной даже на короткой дорожке.
 * Каждая корзина хранит пик всех её отсчётов: короткий звук не исчезает. */
export function fitWave(samples: readonly number[], width: number, scale: number): number[] {
  if (samples.length === 0) return [];
  const count = Math.min(samples.length, Math.max(1, Math.floor((width + 1) / (2 * scale + 1))));
  return Array.from({ length: count }, (_, bin) => {
    const first = Math.floor(bin * samples.length / count);
    const last = Math.floor((bin + 1) * samples.length / count);
    let peak = 0;
    for (let index = first; index < last; index += 1) peak = Math.max(peak, samples[index] ?? 0);
    return peak;
  });
}
