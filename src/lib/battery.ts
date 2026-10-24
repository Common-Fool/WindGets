import { liveSnapshot } from "./live";

export type DeviceKind = "mouse" | "keyboard" | "headset" | "gamepad" | "phone" | "other";
export type DeviceSource = "windows" | "logitech" | "headset";

export type Device = {
  /** Ключ, стабильный между запусками: `bt:…`, `lg:…`, `hs:…`. По нему снимают с карточки. */
  id: string;
  name: string;
  kind: DeviceKind;
  /** Проценты, 0–100. */
  level: number;
  /** `null` — источник не говорит, заряжается ли. */
  charging: boolean | null;
  source: DeviceSource;
};

export type BatteryView = {
  /** Первый опрос прошёл: до него пустой список — «ещё не спросили». */
  ready: boolean;
  /** Только подключённые устройства с известным зарядом; скрытые тоже — снимает карточка. */
  devices: Device[];
};

const EMPTY: BatteryView = { ready: false, devices: [] };

/** Подписка одна на окно и снимается без слушателей (`live.ts`). */
export const useBattery = liveSnapshot<BatteryView>("battery_state", "battery", EMPTY);

/** Ступень заряда — для статусного цвета: больше 25 %, 10–25 %, меньше 10 %. */
export type BatteryStep = "good" | "low" | "critical";

export function batteryStep(level: number): BatteryStep {
  if (level > 25) return "good";
  if (level >= 10) return "low";
  return "critical";
}
