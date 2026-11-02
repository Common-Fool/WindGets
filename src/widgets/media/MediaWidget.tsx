import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GlassCard } from "../../components/GlassCard";
import { PauseIcon, PlayIcon } from "../../components/Transport";
import { usePresetWindowGrowth } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import { useLook, useScale } from "../../lib/look";
import {
  artOf,
  control,
  pickSession,
  seek,
  shown,
  titleOf,
  useMedia,
  type Track,
} from "../../lib/media";
import { useWindowResized } from "../../lib/resized";
import type { Action, Player } from "../../lib/spotify";
import { useWidgetCover, useWidgetLayout, useWidgetSize, useWidgetView } from "../../lib/view";
import { columnGap, PlayerPanel, type Abilities } from "../spotify/PlayerPanel";
import { referenceSlack, sizeKind } from "../spotify/ReferencePlayer";
import "../spotify/spotify.css";
import "../spotify/spotify-reference.css";

/*
 * Медиа: что играет в Windows прямо сейчас — любой плеер, любая вкладка.
 *
 * ВАЖНО: **по виду это пульт Спотифая целиком** (1 ноября 2026, слово
 * пользователя): те же раскладки, обложки, размеры, разметка и CSS —
 * `PlayerPanel` и `ReferencePlayer` общие. Своё у медиа — только данные:
 * сессия переводится в `Player`, команды пульта — в команды сессии. Нижний ряд
 * вместо плейлистов и устройства держит источники: кнопку списка и имя
 * текущего. Перемешивания, повтора и громкости у медиасессии нет.
 */

/** Сколько ждём, что сессия согласится с нажатием пуска или паузы. */
const EXPECT_FOR = 2000;

/** Как часто сверяемся с сессией, пока ждём согласия. */
const EXPECT_STEP = 100;

/**
 * Строка списка источников при 100 % вместе с зазором списка — как у
 * плейлистов (замер: NEW 36 без зазора, Classic 44 и 4).
 */
const ROW = { reference: { row: 36, gap: 0 }, classic: { row: 44, gap: 4 } } as const;

/** Строка громкости Classic с зазором столбца: 17 + 8 (замер при 100 %). */
const CLASSIC_VOLUME_ROW = 25;

/** Больше строк список не растит окно: остальное прокручивается. */
const ROWS_SHOWN = 3;

/** Ничего не играет: кнопкам нечего делать. */
const NOTHING: Abilities = { modes: false, play: false, previous: false, next: false, seek: false };

export function MediaWidget() {
  const t = useText();
  const size = useWidgetSize();
  const widgetView = useWidgetView();
  const layout = useWidgetLayout();
  const cover = useWidgetCover();
  const { design, effectiveDesign } = useLook();
  const reference = effectiveDesign === "reference";
  const geometry = useScale();
  const view = useMedia();
  const track = shown(view);
  const art = useCover(track);
  const [listOpen, setListOpen] = useState(false);
  const blank = useMemo(() => idle(t("media.idle"), t("media.idleHint")), [t]);

  // Размер поменяли — список сворачиваем вместе с ним, как плейлисты
  useWindowResized(() => setListOpen(false));
  const growth = usePresetWindowGrowth(widgetView, size, design, layout);
  const kind = sizeKind(widgetView, size);
  const open = listOpen && view.sessions.length > 0 && kind < 2;

  useEffect(() => {
    if (view.sessions.length === 0) setListOpen(false);
  }, [view.sessions.length]);

  /*
   * Высота по содержимому: громкости у сессии нет, и место под неё окно не
   * держит. У NEW это та же поправка, что у Спотифая без громкости
   * (`referenceSlack`); у Classic — строка громкости с зазором столбца в
   * столбике «Большом» и «Среднем», единственных, где она есть.
   */
  const slack = (reference ? referenceSlack(widgetView, size, cover, false)
    : widgetView === 1 && kind < 2 ? CLASSIC_VOLUME_ROW : 0) * geometry;
  const rows = Math.min(Math.max(view.sessions.length, 1), ROWS_SHOWN);
  const row = reference ? ROW.reference : ROW.classic;
  const panelHeight = (rows * row.row + (rows - 1) * row.gap) * geometry;
  const body = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    growth.open((open ? panelHeight + columnGap(body.current) : 0) - slack);
  }, [design, effectiveDesign, layout, widgetView, size, kind, geometry, open, panelHeight, slack, growth]);

  // «+N» — о других ИГРАЮЩИХ: вкладка на паузе вторым источником не считается
  const others = track === null ? 0
    : view.sessions.filter((session) => session.playing && session.source !== track.source).length;

  return (
    <div className={reference ? "spotify spotify-reference-widget media" : "spotify media"}
      data-size={kind} data-layout="a" data-format={widgetView}>
      <GlassCard accent="media" flush={reference}>
        <div className="spotify__body" ref={body}>
          {track === null ? (
            <PlayerPanel player={blank} receivedAt={0} send={nothing} stale can={NOTHING} />
          ) : (
            // Ключом — чтобы ожидаемое состояние не переезжало с одной сессии
            // на другую при переключении источника
            <Session key={track.source} track={track} art={art} />
          )}

          <div className="spotify__actions">
            <button
              type="button"
              className={open ? "spotify__link spotify__link--active" : "spotify__link"}
              disabled={view.sessions.length === 0}
              onClick={() => setListOpen((was) => !was)}
            >
              {/* Без значка: знак плейлистов тут чужой, а своего у
                  источников нет */}
              {t("media.settings.sources")}
            </button>
            {track !== null && (
              <span className="spotify__device"
                title={others > 0 ? t.count("media.moreSources", others) : undefined}>
                {nameOf(track)}{others > 0 && ` +${others}`}
              </span>
            )}
          </div>

          {open && track !== null && (
            <Sessions sessions={view.sessions} current={track.source} height={panelHeight} />
          )}
        </div>
      </GlassCard>
    </div>
  );
}

/** Одна сессия на пульте: перевод данных и команд в язык пульта. */
function Session({ track, art }: { track: Track; art: string | null }) {
  const t = useText();
  // Согласие сессии ждётся по свежему снимку, а не по тому, что был при нажатии
  const latest = useRef(track);
  latest.current = track;

  /*
   * ВАЖНО: **ожидание пуска/паузы снимает согласие сессии, а не ответ
   * команды.** Команда только кладёт заказ в очередь потока и возвращается
   * сразу, а плеер сообщает о паузе с опозданием — пульт снимает ожидаемое
   * состояние, когда `send` вернулся, и кнопка прыгала бы назад. Поэтому
   * `send` у медиа ждёт, пока сессия согласится, но не дольше двух секунд:
   * не согласилась — нажатие не прошло.
   */
  const send = useCallback(async (action: Action) => {
    const source = latest.current.source;
    try {
      switch (action.kind) {
        case "play":
        case "pause": {
          await control(source, action.kind);
          const wanted = action.kind === "play";
          const until = Date.now() + EXPECT_FOR;
          while (latest.current.playing !== wanted && Date.now() < until) {
            await new Promise((resolve) => window.setTimeout(resolve, EXPECT_STEP));
          }
          return;
        }
        case "next":
        case "previous":
          return await control(source, action.kind);
        case "seek":
          return await seek(source, action.positionMs / 1000);
        default:
          // Перемешивания, повтора, громкости и плейлистов у сессии нет
          return;
      }
    } catch {
      // Отказ сессии виден и так: кнопка возвращается к тому, что она говорит
    }
  }, []);

  const player = useMemo(() => asPlayer(track, art, t("media.unknownTrack")), [track, art, t]);
  const can: Abilities = {
    modes: false,
    play: track.canPlay,
    previous: track.canPrevious,
    next: track.canNext,
    seek: track.canSeek,
  };

  return <PlayerPanel player={player} receivedAt={measuredAt(track)} send={send} can={can} />;
}

/** Сессия в форме пульта Спотифая. */
function asPlayer(track: Track, art: string | null, unknown: string): Player {
  return {
    track: track.song === "" ? unknown : track.song,
    artists: track.artist,
    album: "",
    coverUrl: art,
    link: null,
    isPlaying: track.playing,
    progressMs: Math.round(track.position * 1000),
    durationMs: Math.max(0, Math.round(track.duration * 1000)),
    shuffle: false,
    repeat: "off",
    volume: null,
    device: null,
    supportsVolume: false,
  };
}

async function nothing() {}

/** Пустой пульт: подписи вместо трека, блёклый, без кнопок. */
function idle(title: string, hint: string): Player {
  return {
    track: title, artists: hint, album: "", coverUrl: null, link: null, isPlaying: false,
    progressMs: 0, durationMs: 0, shuffle: false, repeat: "off", volume: null, device: null,
    supportsVolume: false,
  };
}

/** ВАЖНО: **нулевая метка или из будущего значит «сейчас»**, иначе полоса упёрлась бы в конец. */
function measuredAt(track: Track): number {
  const now = Date.now();
  return track.measuredAt > 0 && track.measuredAt <= now ? track.measuredAt : now;
}

function nameOf(track: Track): string {
  return track.title === "" ? track.source : track.title;
}

/**
 * Список источников на месте плейлистов: строка выбирает сессию
 * (`pickSession`), своих команд у неё нет. Высота — то же число, на которое
 * выросло окно (см. `PlaylistPanel`).
 */
function Sessions({ sessions, current, height }: { sessions: Track[]; current: string; height: number }) {
  return (
    <ul className="playlists" style={{ height }}>
      {sessions.map((session) => (
        <li key={session.source}>
          <button
            type="button"
            className="playlists__item"
            aria-pressed={session.source === current}
            title={titleOf(session)}
            onClick={() => void pickSession(session.source)}
          >
            <span className="playlists__cover playlists__cover--empty">
              {session.playing ? <PlayIcon /> : <PauseIcon />}
            </span>
            <span className="playlists__name">{nameOf(session)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/**
 * Обложка по ключу трека.
 *
 * Отдельной командой, а не в снимке: это байты в base64, и гонять их в окно
 * на каждом снимке незачем. Запоздавший ответ о прошлом треке не должен
 * подменить обложку нынешнего — отсюда флаг `alive`.
 */
function useCover(track: Track | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  const source = track?.source ?? null;
  const key = track?.art ?? null;

  useEffect(() => {
    if (source === null || key === null) {
      setUrl(null);
      return;
    }

    let alive = true;
    artOf(source)
      .then((next) => {
        if (alive) setUrl(next);
      })
      .catch(() => {
        if (alive) setUrl(null);
      });

    return () => {
      alive = false;
    };
  }, [source, key]);

  return url;
}
