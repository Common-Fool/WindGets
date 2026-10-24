import type { ConverterView } from "../../lib/converter";
import type { DownloaderView } from "../../lib/downloader";
import type { ToolsView } from "../../lib/tools";
import { NOW, type Answers } from "./common";

/* Программы готовы; пути — вымышленные, заданий в работе нет. */

const TOOLS: ToolsView = {
  components: [
    { id: "ffmpeg", name: "FFmpeg", presence: "installed", version: "8.0",
      downloadBytes: 32 * 1024 ** 2, diskBytes: 96 * 1024 ** 2, checkedAt: Math.floor(NOW / 1000) },
    { id: "ytDlp", name: "yt-dlp", presence: "installed", version: "2026.10.01",
      downloadBytes: 18 * 1024 ** 2, diskBytes: 18 * 1024 ** 2, checkedAt: Math.floor(NOW / 1000) },
    { id: "quickJs", name: "QuickJS", presence: "installed", version: "0.10.1",
      downloadBytes: 2 * 1024 ** 2, diskBytes: 2 * 1024 ** 2, checkedAt: Math.floor(NOW / 1000) },
    { id: "presentMon", name: "PresentMon", presence: "installed", version: "2.6.0",
      downloadBytes: 980_320, diskBytes: 980_320, checkedAt: Math.floor(NOW / 1000) },
  ],
  modules: [
    { id: "converter", needs: ["ffmpeg"], missing: [] },
    { id: "downloader", needs: ["ytDlp", "quickJs", "ffmpeg"], missing: [] },
    { id: "fps", needs: ["presentMon"], missing: [] },
  ],
  job: null,
  failure: null,
  notice: null,
  probing: false,
};

const CONVERTER: ConverterView = {
  items: [{
    id: 1,
    name: "clip.mp4",
    kind: "video",
    duration: 96,
    status: "ready",
    progress: null,
    reason: null,
    output: null,
  }],
  options: {
    target: "mp4",
    quality: "balanced",
    videoSize: 720,
    bitrate: 192,
    gifSize: 480,
    gifFps: 15,
    imageSize: 0,
    from: "",
    to: "",
  },
  targets: ["mp4", "gif", "mp3"],
  target: "mp4",
  sizes: {
    video: { offered: [0, 1080, 720, 480], chosen: 720 },
    gif: { offered: [640, 480, 320], chosen: 480 },
    image: { offered: [0], chosen: 0 },
  },
  mixed: false,
  trim: { enabled: true, duration: 96, error: null, from: null, to: null, kind: "video", subject: 1 },
  running: false,
  overall: null,
  outDir: "C:/Demo/Windgets/Converter",
  outDirCustom: false,
};

const DOWNLOADER: DownloaderView = {
  status: "idle",
  url: null,
  title: null,
  part: null,
  percent: null,
  downloaded: null,
  total: null,
  speed: null,
  eta: null,
  step: null,
  reason: null,
  detail: null,
  output: null,
  mode: "video",
  quality: "p1080",
  outDir: "C:/Demo/Windgets/Downloads",
  outDirCustom: false,
  history: [
    { id: 2, url: "https://example.com/watch/demo-clip", title: "A Short Creative Break", file: "creative-break.mp4", mode: "video" },
    { id: 1, url: "https://example.com/listen/demo-audio", title: "Quiet Workshop", file: "quiet-workshop.mp3", mode: "audio" },
  ],
};

export const tools: Answers = {
  tools_state: () => TOOLS,
  converter_state: () => CONVERTER,
  downloader_state: () => DOWNLOADER,
};
