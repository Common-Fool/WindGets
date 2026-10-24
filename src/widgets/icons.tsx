import type { ComponentType } from "react";
import {
  BatteryIcon,
  BugIcon,
  ChecklistIcon,
  ClipboardIcon,
  CloudSunIcon,
  ConvertIcon,
  CpuIcon,
  DownloadIcon,
  DriveIcon,
  EnvelopeIcon,
  GaugeIcon,
  GpuIcon,
  KeyIcon,
  ListIcon,
  LockIcon,
  MediaIcon,
  MusicIcon,
  NetworkIcon,
  NotepadIcon,
  ShapesIcon,
  FramesIcon,
  SoundIcon,
} from "../components/Icons";

/**
 * Значок каждого виджета — для его строки в дереве колонки окна настроек.
 *
 * Отдельно от реестра `registry.ts`: реестр грузится в каждое окно, а значки
 * нужны только настройкам. Запись есть у каждого виджета реестра — держит это
 * тест `every_widget_has_an_icon` в `widgets.rs`.
 */
export const WIDGET_ICONS: Record<string, ComponentType> = {
  gpu: GpuIcon,
  cpu: CpuIcon,
  disks: DriveIcon,
  battery: BatteryIcon,
  network: NetworkIcon,
  processes: ListIcon,
  fps: FramesIcon,
  weather: CloudSunIcon,
  speedtest: GaugeIcon,
  sound: SoundIcon,
  clipboard: ClipboardIcon,
  notes: NotepadIcon,
  todo: ChecklistIcon,
  mail: EnvelopeIcon,
  password: KeyIcon,
  totp: LockIcon,
  spotify: MusicIcon,
  media: MediaIcon,
  converter: ConvertIcon,
  downloader: DownloadIcon,
  debug: BugIcon,
  kitchensink: ShapesIcon,
};
