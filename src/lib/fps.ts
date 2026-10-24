import { liveSnapshot } from "./live";

/*
 * Счётчик кадров: снимок от PresentMon раз в секунду (`src-tauri/src/fps`).
 * Rust уже выбрал игру и посчитал числа — здесь только показ.
 */

export type FpsStatus = "idle" | "noTool" | "waiting" | "live" | "failed";

export type FpsView = {
  status: FpsStatus;
  /** Имя игры без `.exe`. */
  app: string | null;
  /** Способ вывода: `Hardware: Independent Flip`, `Composed: Flip`… */
  mode: string | null;
  fps: number | null;
  average: number | null;
  low1: number | null;
  low01: number | null;
  frameMs: number | null;
  /** Доля кадра, когда видеокарта занята, %. */
  gpuBusy: number | null;
  latency: number | null;
  dropped: number | null;
  /** FPS по секундам, от старых к новым. */
  history: number[];
  problem: string | null;
};

export const EMPTY_FPS: FpsView = {
  status: "idle",
  app: null,
  mode: null,
  fps: null,
  average: null,
  low1: null,
  low01: null,
  frameMs: null,
  gpuBusy: null,
  latency: null,
  dropped: null,
  history: [],
  problem: null,
};

export const useFps = liveSnapshot<FpsView>("fps_state", "fps", EMPTY_FPS);
