import type { ClipboardState, Entry } from "../../lib/clipboard";
import type { artOf, View as MediaView } from "../../lib/media";
import type { FpsView } from "../../lib/fps";
import type { Group, ProcessesState } from "../../lib/processes";
import type { Icon, SoundState } from "../../lib/sound";
import { GIB, NOW, type Answers } from "./common";

/* Системные карточки: готовый снимок, без значков программ и обложек. */

const GROUPS: Group[] = [
  { name: "chrome", cpu: 12.8, memory: Math.round(2.4 * GIB), count: 18 },
  { name: "Code", cpu: 6.2, memory: Math.round(1.1 * GIB), count: 7 },
  { name: "Discord", cpu: 3.1, memory: 420 * 1024 ** 2, count: 4 },
  { name: "explorer", cpu: 1.7, memory: 180 * 1024 ** 2, count: 1 },
  { name: "steam", cpu: 1.2, memory: 320 * 1024 ** 2, count: 3 },
  { name: "Spotify", cpu: 0.9, memory: 270 * 1024 ** 2, count: 5 },
  { name: "dwm", cpu: 0.7, memory: 96 * 1024 ** 2, count: 1 },
  { name: "SearchHost", cpu: 0.3, memory: 72 * 1024 ** 2, count: 1 },
  { name: "notepad", cpu: 0.1, memory: 24 * 1024 ** 2, count: 1 },
];

const PROCESSES: ProcessesState = {
  byCpu: [...GROUPS].sort((a, b) => b.cpu - a.cpu),
  byMemory: [...GROUPS].sort((a, b) => b.memory - a.memory),
};

/* Игра нейтральная и выдуманная; история — ровная волна с одной просадкой */
const FPS: FpsView = {
  status: "live",
  app: "DemoGame",
  mode: "Hardware: Independent Flip",
  fps: 142,
  average: 138.4,
  low1: 96,
  low01: 71,
  frameMs: 7.04,
  gpuBusy: 97,
  latency: 18.2,
  dropped: 0,
  history: Array.from({ length: 120 }, (_, index) =>
    Math.round(138 + 8 * Math.sin(index / 6) - (index > 80 && index < 86 ? 45 : 0))),
  problem: null,
};

const SOUND: SoundState = {
  output: { volume: 45, muted: false },
  microphone: { muted: false },
  current: { id: "speakers", name: "Speakers", short: "Speakers", kind: "speakers", current: true },
  devices: [
    { id: "speakers", name: "Speakers", short: "Speakers", kind: "speakers", current: true },
    { id: "headphones", name: "Headphones", short: "Headphones", kind: "headphones", current: false },
  ],
  inputs: [
    { id: "microphone", name: "Microphone", short: "Microphone", kind: "other", current: true },
  ],
  sessions: [
    { id: "Spotify", name: "Spotify", volume: 65, muted: false },
    { id: "chrome", name: "chrome", volume: 35, muted: false },
    { id: "Discord", name: "Discord", volume: 80, muted: false },
    { id: "steam", name: "steam", volume: 20, muted: true },
  ],
  canSwitch: true,
  restore: 45,
};

const SOUND_ICON: Icon | null = null;

const MEDIA: MediaView = {
  sessions: [{
    source: "demo-player",
    title: "Media Player",
    song: "Paper Satellites",
    artist: "Velvet Meridian",
    playing: true,
    canPlay: true,
    canNext: true,
    canPrevious: true,
    canSeek: true,
    position: 78,
    duration: 224,
    measuredAt: NOW,
    art: null,
  }],
  current: "demo-player",
  sources: [{ id: "demo-player", title: "Media Player", enabled: true, seenAt: NOW }],
};

const MEDIA_ART: Awaited<ReturnType<typeof artOf>> = null;

function clipboardEntry(id: number, value: string, current = false, pinned = false): Entry {
  return {
    id,
    preview: value.replace(/\s+/g, " ").trim(),
    formattedPreview: value,
    lines: value.split("\n").length,
    chars: value.length,
    current,
    pinned,
  };
}

const CLIPBOARD: ClipboardState = {
  entries: [
    clipboardEntry(6, "https://example.com/guide", true, true),
    clipboardEntry(5, "Small steps make room for good ideas.", false, true),
    clipboardEntry(4, "const total = items.reduce((sum, item) => sum + item.price, 0);"),
    clipboardEntry(3, "Preview checklist\nCheck spacing\nReview contrast\nSave the draft"),
    clipboardEntry(2, "npm run build"),
    clipboardEntry(1, "A quiet afternoon, a fresh page, and a little focus."),
  ],
  capacity: 20,
  images: [
    { id: 104, width: 1920, height: 1080, current: false },
    { id: 103, width: 800, height: 1200, current: false },
    { id: 102, width: 640, height: 640, current: false },
    { id: 101, width: 1200, height: 800, current: false },
  ],
  imageCapacity: 10,
};

/** Нейтральные геометрические миниатюры без текста и внешних ресурсов. */
function clipboardThumbnail(id: number): string | null {
  const image = CLIPBOARD.images.find((entry) => entry.id === id);
  if (!image) return null;
  const width = image.width >= image.height ? 160 : Math.round(160 * image.width / image.height);
  const height = image.height >= image.width ? 160 : Math.round(160 * image.height / image.width);
  const shapes = [
    '<circle cx="45" cy="42" r="25" fill="#a5adb5"/><path d="M55 85L110 20L155 85Z" fill="#737f89"/>',
    '<rect x="18" y="24" width="72" height="88" rx="12" fill="#7e8c97"/><circle cx="62" cy="118" r="24" fill="#b0b8bf"/>',
    '<path d="M80 18L142 80L80 142L18 80Z" fill="#8d99a3"/><circle cx="80" cy="80" r="25" fill="#c0c7cc"/>',
    '<rect x="12" y="18" width="88" height="60" rx="10" fill="#9ca7af"/><path d="M78 28H150V92H78Z" fill="#697985"/>',
  ];
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height +
    '" viewBox="0 0 160 160"><rect width="160" height="160" fill="#30373d"/>' +
    shapes[id - 101] + '</svg>';
  return "data:image/svg+xml," + encodeURIComponent(svg);
}

export const system: Answers = {
  processes_state: () => PROCESSES,
  fps_state: () => FPS,
  sound_state: () => SOUND,
  sound_icon: () => SOUND_ICON,
  media_state: () => MEDIA,
  media_art: () => MEDIA_ART,
  clipboard_state: () => CLIPBOARD,
  clipboard_thumb: ({ id }) => clipboardThumbnail(Number(id)),
};
