import type { Snapshot as SpeedtestSnapshot } from "../../lib/speedtest";
import type { Playlist, SpotifyView } from "../../lib/spotify";
import type { Ahead, Hour, Sky, WeatherState } from "../../lib/weather";
import { DAY, DEMO_PLACE, HOUR, NOW, type Answers } from "./common";

/* Погода и музыка вымышлены; сетевых адресов обложек нет. */

const CITY_DATE = new Intl.DateTimeFormat("en-CA", {
  timeZone: "Europe/Vienna", year: "numeric", month: "2-digit", day: "2-digit",
}).format(NOW);
const CITY_NOON = Date.parse(CITY_DATE + "T12:00:00Z");
const CITY_TIME = new Intl.DateTimeFormat("en-GB", {
  timeZone: "Europe/Vienna", hour: "2-digit", minute: "2-digit", hourCycle: "h23",
});

function hour(at: string, sky: Sky, temperature: number, precipitation: number, wind: number): Hour {
  const h = Number(at.slice(0, 2));
  return { at, sky, temperature, precipitation, wind, day: h >= 7 && h < 19 };
}

function ahead(offset: number, sky: Sky, max: number, min: number, precipitation: number): Ahead {
  return {
    date: new Date(CITY_NOON + offset * DAY).toISOString().slice(0, 10),
    sky,
    day: { max, min, precipitation },
    windows: [
      hour("04:00", "partlyCloudy", min, 5, 1.8),
      hour("08:00", sky, min + 3, precipitation, 2.2),
      hour("12:00", sky, max - 1, precipitation, 3.4),
      hour("16:00", sky, max, precipitation, 3.1),
      hour("20:00", "partlyCloudy", min + 4, 10, 2.0),
    ],
  };
}

const HOURS: Hour[] = [20, 21, 20, 18, 17, 16].map((temperature, index) => {
  const at = CITY_TIME.format(Math.ceil(NOW / HOUR) * HOUR + index * 2 * HOUR);
  return hour(at, index < 3 ? "partlyCloudy" : "clear", temperature, index < 3 ? 10 : 5, 2.5);
});

const WEATHER: WeatherState = {
  status: "ready",
  place: DEMO_PLACE.name,
  stale: false,
  report: {
    temperature: 19,
    sky: "partlyCloudy",
    day: Number(CITY_TIME.format(NOW).slice(0, 2)) >= 7 && Number(CITY_TIME.format(NOW).slice(0, 2)) < 19,
    precipitation: 10,
    wind: 2.8,
    hours: HOURS,
    days: [
      ahead(1, "clear", 22, 12, 5),
      ahead(2, "partlyCloudy", 21, 13, 15),
      ahead(3, "rain", 18, 12, 75),
      ahead(4, "overcast", 19, 11, 25),
      ahead(5, "clear", 23, 12, 5),
    ],
  },
};

// Последний замер только что закончился: в покое без него крупные числа стояли
// бы прочерками (вчерашний замер свежим не притворяется), а превью — витрина
const SPEEDTEST: SpeedtestSnapshot = {
  status: "done",
  report: { download: 492.1, upload: 328.4, latency: 10.6 },
  history: [
    { download: 438.6, upload: 284.1, latency: 14.2 },
    { download: 462.3, upload: 301.7, latency: 12.8 },
    { download: 451.8, upload: 295.4, latency: 13.1 },
    { download: 478.2, upload: 316.8, latency: 11.6 },
    { download: 469.5, upload: 308.2, latency: 12.3 },
    { download: 486.7, upload: 325.6, latency: 10.9 },
    { download: 475.4, upload: 319.3, latency: 11.8 },
    { download: 492.1, upload: 328.4, latency: 10.6 },
  ],
  server: DEMO_PLACE.name,
};

const SPOTIFY: SpotifyView = {
  status: "active",
  player: {
    track: "Lanterns in the Quiet",
    artists: "Amber Circuit",
    album: "Soft Horizons",
    coverUrl: null,
    link: null,
    isPlaying: true,
    progressMs: 86_000,
    durationMs: 238_000,
    shuffle: false,
    repeat: "context",
    volume: 45,
    device: "Desktop",
    supportsVolume: true,
  },
};

const PLAYLISTS: Playlist[] = [
  { id: "demo-focus", name: "Quiet Focus", uri: "spotify:playlist:demo-focus", coverUrl: null, tracks: 24 },
  { id: "demo-evening", name: "Evening Sketches", uri: "spotify:playlist:demo-evening", coverUrl: null, tracks: 18 },
  { id: "demo-weekend", name: "Weekend Windows", uri: "spotify:playlist:demo-weekend", coverUrl: null, tracks: 32 },
];

export const online: Answers = {
  weather_state: () => WEATHER,
  speedtest_view: () => SPEEDTEST,
  spotify_view: () => SPOTIFY,
  spotify_playlists: () => PLAYLISTS,
};
