import type { CSSProperties, ReactNode } from "react";
import type { Action, Player } from "../../lib/spotify";
import { nextRepeat } from "../../lib/spotify";
import { useText } from "../../lib/i18n";
import { NextIcon, PauseIcon, PlayIcon, PreviousIcon } from "../../components/Transport";
import { RepeatIcon, RepeatOneIcon, ShuffleIcon } from "./icons";

/** Числа утверждённого R08 при 100%; native таблицу ведёт widgets.rs. */
export const REFERENCE_SIZES = [
  [[464, 152], [420, 140], [320, 88], [300, 66]],
  [[280, 494], [240, 454], [204, 342], [180, 318]],
  [[320, 320], [280, 280], [240, 240], [210, 210]],
] as const;

export const CLAUDE_SIZES = [
  [[520, 184], [430, 140], [340, 96], [300, 66]],
  [[300, 526], [260, 486], [220, 385], [180, 345]],
  [[360, 360], [310, 310], [260, 260], [210, 210]],
] as const;

type Bounds = { x: number; y: number; w: number; h: number };
type Props = {
  format: number; size: number; cover: number;
  shown: Player; stale: boolean;
  issue: (action: Action, patch?: Partial<Player>) => void;
  art: ReactNode; backdrop: ReactNode; meta: ReactNode;
  progress: ReactNode; edgeProgress: ReactNode; sound: ReactNode;
};

function bounds(x: number, y: number, w: number, h: number): Bounds { return { x, y, w, h }; }
function slotStyle(box: Bounds): CSSProperties {
  const scale = (v: number) => `calc(${v}px * var(--scale))`;
  return { left: scale(box.x), top: scale(box.y), width: scale(box.w), height: scale(box.h) };
}

/** Воздух под последней строкой пульта до черты «Плейлисты», без зазора столбца. */
const FIT_PAD = 6;

/**
 * На сколько точек (при 100 %) пульт ниже своей таблицы — у вертикального
 * «Большого» и «Среднего», где под пультом стоит ряд «Плейлисты».
 *
 * ВАЖНО: **высота у них по содержимому, а не по таблице** (11 октября, слово
 * пользователя). Таблица снята с громкостью и запасом под ней, а громкость
 * бывает не у всякого устройства: у телефона Spotify менять её не даёт. Её
 * место оставалось пустым, и между временем и чертой лежало полсотни точек.
 * Теперь черта встаёт сразу под последней строкой — громкостью или полосой.
 * Число одно на пульт и на окно: пульт ставит по нему свою высоту, а
 * `SpotifyWidget` — прибавку окна. У остальных раскладок и размеров ноль.
 */
export function referenceSlack(format: number, size: number, cover: number, volume: boolean): number {
  const layout = layoutOf(format, size, cover);
  if (layout.format !== 1 || !layout.extra) return 0;
  // Нижняя из строк, какая бы ни стояла последней: порядок строк уже меняли
  const rows = [layout.progress, layout.controls, volume ? layout.sound : null];
  const bottom = Math.max(...rows.map((row) => row === null ? 0 : row.y + row.h));
  return Math.max(0, layout.h - layout.footer - (bottom + FIT_PAD));
}

type Layout = {
  format: number; size: number; nativeH: number; h: number; footer: number; extra: boolean;
  art: Bounds | null; meta: Bounds; progress: Bounds | null; controls: Bounds; sound: Bounds | null;
};

/** Места частей пульта в точках таблицы при 100 %. */
function layoutOf(format: number, size: number, cover: number): Layout {
  const safeFormat = format === 1 || format === 2 ? format : 0;
  const safeSize = Math.min(3, Math.max(0, size));
  const sizes = REFERENCE_SIZES[safeFormat];
  const [nativeW, nativeH] = sizes[safeSize] ?? sizes[0];
  // Поле сцены внутри тонкой рамки. Готовые размеры не растягиваются мышью.
  const w = nativeW - 2, h = nativeH - 2, extra = safeSize < 2;
  const footer = extra ? 28 : 0;
  const picture = safeFormat !== 2 && cover !== 1, big = picture && cover === 2;
  let artBox: Bounds | null = null, metaBox: Bounds, progressBox: Bounds | null = null;
  let controlsBox: Bounds, soundBox: Bounds | null = null;

  if (safeFormat === 0) {
    const side = picture ? big ? h : h - footer - 24 : 0;
    if (picture) artBox = bounds(big ? 0 : 12, big ? 0 : 12, side, side);
    const mx = picture ? side + (big ? 12 : 24) : 12, mw = w - mx - 12;
    if (safeSize === 0) {
      metaBox = bounds(mx, 12, mw, 38);
      controlsBox = bounds(mx + (mw - 186) / 2, 58, 186, 42);
      progressBox = bounds(mx, 101, mw, 26);
    } else if (extra) {
      const x = w - 198, textW = Math.max(60, x - mx - 8);
      metaBox = bounds(mx, 14, textW, 38); controlsBox = bounds(x, 32, 186, 42);
      progressBox = bounds(mx, 67, textW, 26);
    } else {
      const x = w - 112;
      metaBox = bounds(mx, safeSize === 3 ? 12 : 10, safeSize === 3 ? Math.max(64, x - mx - 8) : mw, 38);
      controlsBox = bounds(x, safeSize === 3 ? 17 : 48, 100, 32);
      if (safeSize !== 3) progressBox = bounds(mx, 64, Math.max(40, x - mx - 10), 14);
    }


  } else if (safeFormat === 1) {
    if (picture) { const side = big ? w : w - 24; artBox = bounds(big ? 0 : 12, big ? 0 : 12, side, side); }
    const my = picture ? big ? w + 12 : w : Math.max(12, h - (extra ? 214 : 138));
    // Полоса НАД кнопками, как в столбике Claude (11 октября, слово
    // пользователя): подписи, полоса со временем, кнопки, громкость.
    // Воздух между строками добавлен с живой проверки, когда у «Большого» и
    // «Среднего» высота стала по содержимому и строки слиплись
    metaBox = bounds(12, my, w - 24, 38);
    progressBox = bounds(12, my + (extra ? 46 : 44), w - 24, 26);
    controlsBox = bounds((w - (extra ? 186 : 108)) / 2, my + (extra ? 80 : 76), extra ? 186 : 108, extra ? 42 : 36);
    if (extra) soundBox = bounds(12, my + 132, w - 24, 24);


  } else {
    metaBox = bounds(12, 16, w - 24, 38);
    controlsBox = bounds((w - (extra ? 186 : 108)) / 2, Math.round(h * (extra ? .4 : .43)), extra ? 186 : 108, extra ? 42 : 36);
    progressBox = bounds(12, extra ? Math.round(h * .4) + 51 : h - (safeSize === 3 ? 28 : 54), w - 24, safeSize === 3 ? 14 : 26);
    if (extra) soundBox = bounds(12, h - 75, w - 24, 24);


  }

  return { format: safeFormat, size: safeSize, nativeH, h, footer, extra,
    art: artBox, meta: metaBox, progress: progressBox, controls: controlsBox, sound: soundBox };
}

/** Только расстановка готовых частей. Данные и эффекты остаются у PlayerPanel. */
export function ReferencePlayer({ format, size, cover, shown, issue,
  stale, art, backdrop, meta, progress, edgeProgress, sound }: Props) {
  const layout = layoutOf(format, size, cover);
  const { format: safeFormat, size: safeSize, nativeH, footer, extra } = layout;
  const slack = referenceSlack(format, size, cover, sound !== null);

  return <div className="spotify-reference__player" data-format={safeFormat} data-layout="a"
    data-cover={safeFormat === 2 ? 1 : cover} data-size={safeSize} data-stale={stale || undefined}
    style={{ "--spotify-reference-height": `${nativeH - slack}px`,
      "--spotify-reference-footer": `${footer}px` } as CSSProperties}>
    {backdrop}
    {layout.art && <div className="spotify-reference__slot spotify-reference__art" style={slotStyle(layout.art)}>{art}</div>}
    <div className="spotify-reference__slot spotify-reference__meta" style={slotStyle(layout.meta)}>{meta}</div>
    <div className="spotify-reference__slot spotify-reference__transport"
      data-extra={extra || undefined} style={slotStyle(layout.controls)}>
      <ReferenceTransport shown={shown} issue={issue} modes={extra}
        playSize={extra ? 42 : safeFormat === 0 ? 32 : 36} />
    </div>
    {layout.progress && <div className="spotify-reference__slot spotify-reference__progress"
      data-time={layout.progress.h > 14 || undefined} style={slotStyle(layout.progress)}>{progress}</div>}
    {safeFormat === 0 && safeSize === 3 && cover === 1 &&
      <div className="spotify-reference__edge">{edgeProgress}</div>}
    {layout.sound && sound && <div className="spotify-reference__slot spotify-reference__volume" style={slotStyle(layout.sound)}>{sound}</div>}
  </div>;
}

type CommandProps = { shown: Player; issue: Props["issue"] };
function ModeButton({ kind, shown, issue }: CommandProps & { kind: "shuffle" | "repeat" }) {
  const t = useText();
  const shuffle = kind === "shuffle", active = shuffle ? shown.shuffle : shown.repeat !== "off";
  const label = shuffle ? t("spotify.shuffle") : t(shown.repeat === "track" ? "spotify.repeatTrack" : shown.repeat === "context" ? "spotify.repeatList" : "spotify.repeatOff");
  return <button type="button" className={`player__button${active ? " player__button--active" : ""}`}
    aria-pressed={active} title={label} aria-label={label}
    onClick={() => shuffle ? issue({ kind: "shuffle", on: !shown.shuffle }, { shuffle: !shown.shuffle })
      : issue({ kind: "repeat", mode: nextRepeat(shown.repeat) }, { repeat: nextRepeat(shown.repeat) })}>
    {shuffle ? <ShuffleIcon /> : shown.repeat === "track" ? <RepeatOneIcon /> : <RepeatIcon />}
  </button>;
}
function ReferenceTransport({ shown, issue, modes, playSize }: CommandProps & {
  modes: boolean; playSize: number;
}) {
  const t = useText();
  return <div className="spotify-reference__commands"
    style={{ "--spotify-play-size": `${playSize}px` } as CSSProperties}>
    {modes && <ModeButton shown={shown} issue={issue} kind="shuffle" />}
    <button type="button" className="player__button spotify-reference__previous" title={t("transport.previous")}
      aria-label={t("transport.previous")} onClick={() => issue({ kind: "previous" })}><PreviousIcon /></button>
    <button type="button" className="player__play" title={t(shown.isPlaying ? "transport.pause" : "transport.play")}
      aria-label={t(shown.isPlaying ? "transport.pause" : "transport.play")}
      onClick={() => issue({ kind: shown.isPlaying ? "pause" : "play" }, { isPlaying: !shown.isPlaying })}>
      {shown.isPlaying ? <PauseIcon /> : <PlayIcon />}
    </button>
    <button type="button" className="player__button spotify-reference__next" title={t("transport.next")}
      aria-label={t("transport.next")} onClick={() => issue({ kind: "next" })}><NextIcon /></button>
    {modes && <ModeButton shown={shown} issue={issue} kind="repeat" />}
  </div>;
}
