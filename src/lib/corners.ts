import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/*
 * Прямые углы карточки по соседям (`corners.rs`): стык двух виджетов прямой,
 * свободный край скруглён. Rust считает и шлёт адресно; здесь углы ложатся
 * атрибутом `data-square` на корень, а гасит скругление `base.css`.
 */

/** Порядок тот же, что у `Square` в Rust: верх-лево, верх-право, низ-право, низ-лево. */
const NAMES = ["tl", "tr", "br", "bl"] as const;

function apply(square: boolean[]) {
  const names = NAMES.filter((_, index) => square[index] === true);
  if (names.length === 0) delete document.documentElement.dataset.square;
  else document.documentElement.dataset.square = names.join(" ");
}

/** Только для окон виджетов: зовёт оболочка (`main.tsx`). */
export function useCorners(enabled: boolean) {
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const ask = () => invoke<boolean[]>("window_corners").then((square) => { if (alive) apply(square); }).catch(() => {});
    let stop: (() => void) | null = null;
    // Спрашиваем уже после подписки: событие, ушедшее между ответом и
    // подпиской, терялось, и окно «забывало» соседа (2 ноября 2026)
    try {
      getCurrentWebviewWindow().listen<boolean[]>("corners", (event) => apply(event.payload))
        .then((unlisten) => {
          if (!alive) { unlisten(); return; }
          stop = unlisten;
          void ask();
        })
        .catch(() => { void ask(); });
    } catch {
      // Вне Tauri (превью, браузер) окна нет — углы остаются круглыми
    }
    return () => {
      alive = false;
      stop?.();
    };
  }, [enabled]);
}
