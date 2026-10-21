import { useEffect, useRef, useState } from "react";
import { GlassCard } from "../../components/GlassCard";
import { Marquee } from "../../components/Marquee";
import { NextIcon, PauseIcon, PlayIcon, PreviousIcon } from "../../components/Transport";
import { useWindowDrag } from "../../lib/drag";
import { useContentHeight } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useSeekDrag } from "../../lib/seek";
import {
  artOf,
  clock,
  control,
  pickSession,
  playedAt,
  seek,
  shown,
  useMedia,
  type Track,
} from "../../lib/media";
import { useWidgetLayout, useWidgetSize } from "../../lib/view";
import { ReferenceIdle, ReferencePlayer, ReferenceSessions } from "./MediaReference";
import "./media.css";
import "./media-reference.css";

/*
 * Медиа: что играет в Windows прямо сейчас — любой плеер, любая вкладка.
 *
 * Раскладка Claude одна: это горизонтальный пульт Спотифая с
 * обложкой обычного размера. Фоновой и крупной обложек тут нет, как и других
 * раскладок, — виджет делается для полного комплекта, а образцом того, что
 * он умеет, служит всплывающее окно медиа самой Windows: источник, название,
 * исполнитель, обложка и три кнопки.
 *
 * Кнопки гасятся по флагам, которые сессия сообщает сама (`canPlay` и
 * соседи), а не по надежде: веб-плеер на рекламе, например, «следующий» не
 * принимает, и кнопка, нажатие которой ничего не делает, читается как поломка.
 */

/** Как часто досчитываем позицию между снимками. Глазу этого достаточно. */
const TICK_MS = 250;

/** Сколько ждём, что сессия согласится с нажатием пуска или паузы. */
const EXPECT_FOR = 2000;

/** Досчёт ближе этого (в секундах) к отпущенному месту — перемотка дошла. */
const SEEK_CLOSE = 2;

/**
 * Номер самого тесного размера. На нём полосы перемотки нет вовсе — ради
 * этого окно там и стало на строку ниже; ровно как у пульта Спотифая.
 */
const SMALLEST_SIZE = 3;

export function MediaWidget() {
  useWindowDrag();

  const size = useWidgetSize();
  const layout = useWidgetLayout() === 1 ? 1 : 0;
  const { effectiveDesign, scale } = useLook();
  const reference = effectiveDesign === "reference";
  const t = useText();
  const view = useMedia();
  const track = shown(view);
  const content = useRef<HTMLDivElement>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const panelOpen = reference && sourcesOpen && view.sessions.length > 1;

  useEffect(() => {
    if (view.sessions.length === 0) setSourcesOpen(false);
  }, [view.sessions.length]);

  // Один владелец геометрии GPT: основа не зависит от высоты окна,
  // список меряется вместе с ней. При смене дизайна отложенный fit отменяется.
  useContentHeight(content, reference,
    [layout, size, panelOpen, Math.min(view.sessions.length, 3), scale, t.language].join("|"), true);

  return (
    <div className={reference ? "widget media media-reference-widget" : "widget media"}
      data-size={size} data-layout={reference ? layout : undefined}>
      <GlassCard accent="media" flush={reference}>
        {/* Постоянный узел сохраняет экземпляр Player при GPT/Claude.
            В Claude display:contents оставляет исходную геометрию ряда. */}
        <div className={reference ? "media-content media-reference" : "media-content"} ref={content}>
        {track === null ? (
          <Idle reference={reference} />
        ) : (
          <Player
            // Ключом — чтобы ожидаемое состояние и обложка не переезжали
            // с одной сессии на другую при переключении источника
            key={track.source}
            track={track}
            size={size}
            reference={reference}
            layout={layout}
            opened={panelOpen}
            onOpen={() => setSourcesOpen((open) => !open)}
            // «+N» — о других ИГРАЮЩИХ: вкладка на паузе вторым источником
            // не считается, иначе подсказка «ещё играет» врала бы почти всегда
            others={view.sessions.filter((session) => session.playing && session.source !== track.source).length}
            canSwitch={view.sessions.some((session) => session.source !== track.source)}
            onSwitch={() => {
              const at = view.sessions.findIndex((session) => session.source === track.source);
              const next = view.sessions[(at + 1) % view.sessions.length];
              if (next !== undefined) void pickSession(next.source);
            }}
          />
        )}
        {panelOpen && track !== null && <ReferenceSessions sessions={view.sessions} current={track.source} />}
        </div>
      </GlassCard>
    </div>
  );
}

/**
 * Ничего не играет.
 *
 * Карточка при этом не пропадает и не схлопывается: окно висит на обоях, и
 * исчезающая карточка двигала бы взгляд. Раскладка та же, что у плеера, —
 * блёклая и без кнопок, которым нечего делать.
 */
function Idle({ reference }: { reference: boolean }) {
  const t = useText();
  if (reference) return <ReferenceIdle />;
  return (
    <div className="media__row media__row--idle">
      <Cover url={null} />
      <div className="media__lane">
        <div className="media__meta">
          <span className="media__song">{t("media.idle")}</span>
          <span className="media__artist">{t("media.idleHint")}</span>
        </div>
      </div>
    </div>
  );
}

function Player({
  track,
  size,
  reference,
  layout,
  opened,
  onOpen,
  others,
  canSwitch,
  onSwitch,
}: {
  track: Track;
  size: number;
  reference: boolean;
  layout: number;
  opened: boolean;
  onOpen: () => void;
  others: number;
  canSwitch: boolean;
  onSwitch: () => void;
}) {
  const t = useText();
  /*
   * Нажатие должно отзываться мгновенно, а новый снимок придёт через сотни
   * миллисекунд. До него показываем ожидаемое.
   *
   * ВАЖНО: **ожидание снимает согласие сессии, а не любой снимок.** Плеер
   * сообщает о паузе с опозданием, и снимок, пришедший сразу после нажатия,
   * ещё говорит «играет» — сброс по нему вернул бы кнопку назад на полсекунды.
   * Не согласилась за две секунды — значит, нажатие не прошло, и карточка
   * возвращается к тому, что говорит сессия.
   */
  const [expected, setExpected] = useState<boolean | null>(null);
  useEffect(() => {
    if (expected === null) return;
    if (track.playing === expected) {
      setExpected(null);
      return;
    }
    const timer = window.setTimeout(() => setExpected(null), EXPECT_FOR);
    return () => window.clearTimeout(timer);
  }, [expected, track.playing]);
  const playing = expected ?? track.playing;

  const position = useTicking(track);
  const cover = useCover(track);

  const known = track.duration > 0;
  const ratio = known ? Math.min(position / track.duration, 1) : 0;

  function toggle() {
    const action = playing ? "pause" : "play";
    setExpected(!playing);
    control(track.source, action).catch(() => setExpected(null));
  }

  // Полосу тянут мышью, отпущенное место держится до ответа (`lib/seek.ts`):
  // перемотку виджет переносит сам (`moved` в Rust), но досчёт до свежего
  // снимка шёл от старого места, и полоса скакала
  const drag = useSeekDrag(ratio, track.duration, SEEK_CLOSE, track.canSeek && known,
    (share) => void seek(track.source, share * track.duration), `${track.song}
${track.artist}`);
  const shownPosition = drag.chosen === null ? position : drag.chosen * track.duration;

  const meta = (
    <div className="media__meta">
      <Source track={track} others={others} canSwitch={canSwitch} onSwitch={onSwitch} />
      <Marquee className="media__song" text={track.song === "" ? t("media.unknownTrack") : track.song} />
      {track.artist !== "" && <Marquee className="media__artist" text={track.artist} />}
    </div>
  );

  const bar = (
    <div className="media__progress">
      <div
        className="media__bar"
        data-seek={track.canSeek && known}
        data-no-drag
        {...drag.handlers}
        role="presentation"
        title={track.canSeek && known ? t("media.seek") : undefined}
      >
        <div className="media__fill" style={{ width: `${(drag.shown * 100).toFixed(2)}%` }} />
      </div>
      {/* Длины нет — нет и чисел. Ряд при этом остаётся на месте, иначе
          карточка прыгала бы на каждом ролике: подробности в media.css */}
      <div className="media__times tabular" data-known={known}>
        <span>{clock(shownPosition)}</span>
        <span>{clock(track.duration)}</span>
      </div>
    </div>
  );

  const buttons = (
    <div className="media__controls">
      <button
        type="button"
        className="media__button"
        title={t("transport.previous")}
        disabled={!track.canPrevious}
        onClick={() => void control(track.source, "previous")}
      >
        <PreviousIcon />
      </button>
      <button
        type="button"
        className="media__play"
        title={playing ? t("transport.pause") : t("transport.play")}
        disabled={!track.canPlay}
        onClick={toggle}
      >
        {playing ? <PauseIcon /> : <PlayIcon />}
      </button>
      <button
        type="button"
        className="media__button"
        title={t("transport.next")}
        disabled={!track.canNext}
        onClick={() => void control(track.source, "next")}
      >
        <NextIcon />
      </button>
    </div>
  );

  /*
   * На самом просторном размере кнопки стоят в середине, под подписями и над
   * полосой: справа они уводили бы взгляд от названия. На остальных они
   * справа — иначе два верхних размера отличались бы одной шириной, и выбор
   * между ними ничего бы не значил. Решение взято у пульта Спотифая целиком.
   */
  // Данные, обложка, досчёт и expected остаются в этом же Player:
  // смена дизайна/A/B не начинает медиасессию заново.
  if (reference) {
    return <ReferencePlayer track={track} size={size} layout={layout} cover={cover}
      playing={playing} position={position} others={others} canSwitch={canSwitch}
      opened={opened} onSwitch={onSwitch} onOpen={onOpen} onToggle={toggle} seek={drag} />;
  }

  if (size === 0) {
    return (
      <div className="media__row">
        <Cover url={cover} />
        <div className="media__lane">
          {meta}
          {buttons}
          {bar}
        </div>
      </div>
    );
  }

  return (
    <div className="media__row">
      <Cover url={cover} />
      <div className="media__lane">
        {meta}
        {size !== SMALLEST_SIZE && bar}
      </div>
      {buttons}
    </div>
  );
}

/**
 * Имя источника — строкой над названием, как во всплывающем окне Windows.
 *
 * Другую сессию можно выбрать и на паузе: «+N» считает лишь играющие,
 * а кнопка листает все доступные. Она маленькая намеренно: удерживает мышь
 * и во всю ширину отняла бы у карточки перетаскивание окна.
 */
function Source({
  track,
  others,
  canSwitch,
  onSwitch,
}: {
  track: Track;
  others: number;
  canSwitch: boolean;
  onSwitch: () => void;
}) {
  const t = useText();
  const name = track.title === "" ? track.source : track.title;

  if (!canSwitch) {
    return <span className="media__source">{name}</span>;
  }

  return (
    <button
      type="button"
      className="media__source media__source--switch"
      title={others > 0 ? t.count("media.moreSources", others) : t("media.settings.sources")}
      onClick={onSwitch}
    >
      {name}
      {others > 0 && <span className="media__more">+{others}</span>}
    </button>
  );
}

/**
 * Обложка слева квадратом во всю высоту ряда.
 *
 * Своей высоты не назначает: её задаёт ряд, а ширину — соотношение сторон.
 * Нет картинки — на её месте та же плашка со значком ноты, чтобы подписи не
 * прыгали влево, когда обложка приходит на полсекунды позже трека.
 */
function Cover({ url }: { url: string | null }) {
  return (
    <div className="media__art">
      {url === null ? (
        <div className="media__cover media__cover--empty">
          <NoteIcon />
        </div>
      ) : (
        <img className="media__cover" src={url} alt="" draggable={false} />
      )}
    </div>
  );
}

/** Нота на пустой обложке. Своя: второго хозяина у неё нет. */
function NoteIcon() {
  return (
    <svg viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true" fill="currentColor">
      <path d="M17 3.5v11.2a3.3 3.3 0 1 1-2-3V7.4L9.5 8.6v8.1a3.3 3.3 0 1 1-2-3V6.8a1 1 0 0 1 .78-.98l7.5-1.66A1 1 0 0 1 17 5.14Z" />
    </svg>
  );
}

/**
 * Позиция, досчитанная между снимками.
 *
 * Rust присылает её только при изменениях, а полоса обязана ехать плавно.
 * Тикает только играющая сессия: у стоящей позиция и так та, что прислали.
 */
function useTicking(track: Track): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setNow(Date.now());
    if (!track.playing) return;

    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, [track.playing, track.measuredAt]);

  return playedAt(track, now);
}

/**
 * Обложка по ключу трека.
 *
 * Отдельной командой, а не в снимке: это байты в base64, и гонять их в окно
 * на каждом снимке незачем. Запоздавший ответ о прошлом треке не должен
 * подменить обложку нынешнего — отсюда флаг `alive`.
 */
function useCover(track: Track): string | null {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (track.art === null) {
      setUrl(null);
      return;
    }

    let alive = true;
    artOf(track.source)
      .then((next) => {
        if (alive) setUrl(next);
      })
      .catch(() => {
        if (alive) setUrl(null);
      });

    return () => {
      alive = false;
    };
  }, [track.source, track.art]);

  return url;
}
