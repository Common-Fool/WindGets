import type { ClipboardState, Entry } from "../../lib/clipboard";
import type { artOf, View as MediaView } from "../../lib/media";
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

function clipboardEntry(id: number, value: string, current = false): Entry {
  return {
    id,
    preview: value.replace(/\s+/g, " ").trim(),
    formattedPreview: value,
    lines: value.split("\n").length,
    chars: value.length,
    current,
  };
}

const CLIPBOARD: ClipboardState = {
  entries: [
    clipboardEntry(6, "https://example.com/guide", true),
    clipboardEntry(5, "Small steps make room for good ideas."),
    clipboardEntry(4, "const total = items.reduce((sum, item) => sum + item.price, 0);"),
    clipboardEntry(3, "Preview checklist\nCheck spacing\nReview contrast\nSave the draft"),
    clipboardEntry(2, "npm run build"),
    clipboardEntry(1, "A quiet afternoon, a fresh page, and a little focus."),
  ],
  capacity: 20,
};

export const system: Answers = {
  processes_state: () => PROCESSES,
  sound_state: () => SOUND,
  sound_icon: () => SOUND_ICON,
  media_state: () => MEDIA,
  media_art: () => MEDIA_ART,
  clipboard_state: () => CLIPBOARD,
};
