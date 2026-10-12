import { invoke } from "@tauri-apps/api/core";
import type { ModuleId } from "./tools";

/*
 * Папки вывода конвертера и загрузчика. По умолчанию — «Рабочий стол\WinGets\…»,
 * своя выбирается окном Windows в настройках виджета.
 */

export function openOutputDir(module: ModuleId): Promise<void> {
  return invoke("open_output_dir", { module });
}

export function pickOutputDir(module: ModuleId): Promise<void> {
  return invoke("pick_output_dir", { module });
}

export function resetOutputDir(module: ModuleId): Promise<void> {
  return invoke("reset_output_dir", { module });
}
