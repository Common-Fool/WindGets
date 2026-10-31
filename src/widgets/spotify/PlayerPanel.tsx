import { useEffect, useMemo, useRef, useState } from "react";
import {
  formatTime,
  nextRepeat,
  positionNow,
  type Action,
  type Player,
} from "../../lib/spotify";
import { useWidgetCover, useWidgetSize, useWidgetView } from "../../lib/view";
import { useLook } from "../../lib/look";
import { useSeekDrag } from "../../lib/seek";
import { ReferencePlayer } from "./ReferencePlayer";
import { Marquee } from "../../components/Marquee";
import { Slider } from "../../components/Slider";
import { NextIcon, PauseIcon, PlayIcon, PreviousIcon } from "../../components/Transport";
import {
  RepeatIcon,
  RepeatOneIcon,
  ShuffleIcon,
  SpotifyMark,
  VolumeIcon,
} from "./icons";
import { useText, type Key } from "../../lib/i18n";
import { openLink } from "../../lib/links";

/** Как часто досчитываем позицию между опросами. Глазу этого достаточно. */
const TICK_MS = 250;

/** Пауза перед отправкой громкости: за перетаскивание ползунка иначе
 *  улетел бы десяток запросов подряд. */
const VOLUME_DEBOUNCE_MS = 200;

/** Сколько после своей отправки не верим громкости из опроса: Spotify
 *  какое-то время отдаёт прежнюю, и ползунок прыгал бы назад. */
const VOLUME_HOLD_MS = 2500;

/**
 * Раскладка: чем окно стоит. Порядок тот же, что у `views` в реестре Rust.
 *
 * Размеры окна у каждой свои — форма у них разная целиком, а не высотой.
 */
export const LAYOUT = { row: 0, column: 1, square: 2 } as const;

/**
 * Чем в карточке служит обложка. Порядок тот же, что у `covers` в реестре.
 *
 * ВАЖНО: **ось независимая, и размера окна она не меняет.** Любая обложка
 * сочетается с любой раскладкой и любым размером — карточка подстраивается под
 * неё сама. Перемножь мы их, вышло бы сорок восемь размеров вместо двенадцати,
 * и каждый пришлось бы снимать замером.
 */
const COVER = { picture: 0, back: 1, big: 2 } as const;

const LAYOUT_CLASS: Record<number, string> = {
  [LAYOUT.row]: "player--row",
  [LAYOUT.column]: "player--column",
  [LAYOUT.square]: "player--square",
};

type Props = {
  player: Player;
  receivedAt: number;
  send: (action: Action) => Promise<void>;
  /** Трек не играет, а вспомнен: показываем блёкло и без громкости. */
  stale?: boolean;
};

/**
 * Номер самого тесного размера.
 *
 * Он особенный в двух раскладках из трёх, и по-разному. В полосе полосы
 * перемотки нет вовсе, и окно от этого на строку ниже; исключение — обложка
 * фоном, у которой полоса ложится ниткой по нижнему краю и высоты не
 * занимает. В квадрате такая нитка стоит всегда, а строки над ней разложены
 * по стороне, а не сбиты к низу.
 */
const SMALLEST_SIZE = 3;

/**
 * С какого размера квадрат складывается по-другому.
 *
 * Выше него столбик стоит вверху карточки, подписями по центру; с него —
 * прижат к низу и выровнен по левому краю. Граница проходит там, где уходит
 * громкость: без неё строк остаётся три, и вверху они смотрятся заголовком
 * над пустотой.
 */
const SMALL_SQUARE = 2;

/** Досчёт ближе этого к отпущенному месту — значит, перемотка дошла. */
const SEEK_CLOSE_MS = 2000;

/**
 * Есть ли у пульта ползунок громкости. У части устройств — телефонов,
 * некоторых колонок — Spotify менять громкость через API не даёт.
 */
export function hasVolume(player: Player): boolean {
  return player.supportsVolume && player.volume !== null;
}

export function PlayerPanel({ player, receivedAt, send, stale = false }: Props) {
  const layout = useWidgetView();
  const size = useWidgetSize();
  const { effectiveDesign } = useLook();

  /*
   * ВАЖНО: **в квадрате обложка бывает только фоном.**
   *
   * Остальные её виды там перепробованы и отвергнуты живой проверкой: у
   * квадрата высота равна ширине, и картинка отдельным блоком либо съедает
   * подписи, либо сама съёживается до значка. Фоном же она занимает карточку
   * целиком, а подписи ложатся поверх — это единственная раскладка, которой
   * квадратная форма идёт на пользу.
   *
   * Настройка при этом не запрещается, а исчезает: в меню и в настройках
   * виджета список обложек у квадрата не показывается вовсе (`covers_hidden`
   * в реестре).
   */
  const chosen = useWidgetCover();
  const cover = layout === LAYOUT.square ? COVER.back : chosen;

  /*
   * Нажатие должно отзываться мгновенно, а подтверждение от Spotify придёт
   * через сотни миллисекунд. До него показываем ожидаемое состояние.
   */
  const [expected, setExpected] = useState<Partial<Player> | null>(null);
  // Досчёт слушает показанное состояние: сохраняем его ссылку между тиками,
  // чтобы ожидание «играть» не перезапускало таймер при каждой перерисовке.
  const shown = useMemo(() => expected ? { ...player, ...expected } : player, [player, expected]);

  // Номер последнего нажатия: ожидание снимает только оно. Иначе первая из
  // двух быстрых команд, вернувшись, стирала бы ожидание второй, ещё бегущей,
  // и кнопка мигала бы опрошенным состоянием
  const issued = useRef(0);

  async function issue(action: Action, patch?: Partial<Player>) {
    const number = ++issued.current;
    if (patch) setExpected(patch);
    try {
      await send(action);
    } finally {
      // К этому моменту бэкенд уже переспросил Spotify и разослал свежее
      if (issued.current === number) setExpected(null);
    }
  }

  const position = useTickingPosition(shown, receivedAt);
  const volume = useVolume(player, send);

  const elapsed = Math.min(position, player.durationMs);
  const ratio = player.durationMs > 0 ? elapsed / player.durationMs : 0;

  function seek(share: number) {
    if (player.durationMs <= 0) return;

    const clamped = Math.min(Math.max(share, 0), 1);
    void issue({ kind: "seek", positionMs: Math.round(clamped * player.durationMs) });
  }

  /*
   * Готовые куски: раскладок три, обложек три, а логика у всех сочетаний
   * одна. Собирается она здесь, до ветвления, ровно затем, чтобы
   * раскладке досталось только расставить готовое — иначе досчёт позиции,
   * ожидаемое состояние и задержка громкости расползлись бы по веткам, и
   * починка в одной не доезжала бы до остальных.
   */
  const art =
    cover === COVER.back ? null : <Cover url={player.coverUrl} big={cover === COVER.big} link={player.link} />;

  const backdrop = cover === COVER.back ? <Backdrop url={player.coverUrl} /> : null;

  const bar = (
    <Progress elapsed={elapsed} total={player.durationMs} ratio={ratio} onSeek={seek} subject={`${player.track}
${player.artists}`} />
  );

  /*
   * Та же полоса ниткой по нижнему краю карточки, поверх обложки.
   *
   * ВАЖНО: **`edge` — это отдельный элемент, а не правило по раскладке.**
   * Прежде ниткой становилась обычная полоса, стоящая внутри `.player__lane`, —
   * а лента у обложки фоном делает `relative` каждого прямого ребёнка пульта,
   * и `bottom: 0` отмерялось от середины карточки, где стоит сама дорожка.
   * Полоса ложилась ровно на кнопки. Ниткой она обязана быть ребёнком пульта,
   * а не дорожки.
   */
  const edgeBar = (
    <Progress edge elapsed={elapsed} total={player.durationMs} ratio={ratio} onSeek={seek} subject={`${player.track}
${player.artists}`} />
  );

  /*
   * Ряд всегда полный, а лишнее убирает CSS.
   *
   * Прежде короткий ряд выбирался тут по раскладке, и перемешивание с повтором
   * пропадали в полосе и квадрате навсегда — на любом размере. Сколько кнопок
   * влезло, знает не раскладка, а размер окна, и решать это его дело.
   */
  const buttons = <Transport shown={shown} issue={issue} />;

  /*
   * Громкость стоит и у вспомненного трека.
   *
   * Прежде её там гасили — «устройства нет, менять не на чем». Довод не
   * выдержал сравнения с соседями: кнопки перемотки у вспомненного трека
   * рабочие, и «играть» само выбирает устройство. Гасить одну громкость
   * значило держать два разных правила для одной и той же карточки, а снаружи
   * это выглядит как пропавший ползунок — место под него в окне остаётся, ведь
   * высота размера снята вместе с ним.
   *
   * Не пройдёт команда — придёт отказ строкой поверх карточки, как у любой
   * другой. Это честнее молчаливого исчезновения.
   */
  // Условие то же, что у `hasVolume` в SpotifyWidget: по нему окно NEW встаёт
  // высотой, и разойдись они — черта «Плейлисты» наехала бы на ползунок
  const sound =
    hasVolume(player) && volume.value !== null ? (
      <VolumeRow value={volume.value} onChange={volume.change} />
    ) : null;

  /*
   * ВАЖНО: **подсказки «включите что-нибудь в Спотифае» тут нет и не будет.**
   *
   * Она стояла строкой под пультом, отнимала две строки высоты — в квадрате
   * это треть обложки — и висела постоянно, ради текста, который читают один
   * раз в жизни. Вынос её поверх карточки дела не поправил: она всё равно
   * закрывала кнопки. То, что ничего не играет, видно и так: пульт блёклый
   * (`player--stale`) и громкости у него нет.
   *
   * Отказы это не отменяет — они живут поверх карточки восемь секунд и
   * сообщают о том, чего человек не мог знать заранее.
   */

  const classes = ["player", LAYOUT_CLASS[layout] ?? LAYOUT_CLASS[LAYOUT.row]];
  if (backdrop !== null) classes.push("player--art");
  if (stale) classes.push("player--stale");
  const root = classes.join(" ");

  // Один PlayerPanel владеет ожидаемым состоянием, досчётом и громкостью.
  // A/B и дизайн меняют только содержимое: таймеры и защита от отката живут дальше.
  if (effectiveDesign === "reference") {
    return <ReferencePlayer format={layout} size={size} cover={cover}
      shown={shown} issue={issue} stale={stale} art={art} backdrop={backdrop}
      meta={<Meta player={player} />} progress={bar} edgeProgress={edgeBar} sound={sound} />;
  }

  /*
   * Квадрат — столбик поверх обложки. Отдельной картинки тут нет ни на одном
   * размере: обложка бывает только фоном, и делить сторону пополам не на что.
   *
   * ВАЖНО: **у квадрата три композиции на четыре размера, и это не небрежность.**
   * Сторона равна стороне, поэтому размер отнимает не ширину, а строки, — и
   * место, где столбик стоит, от их числа зависит сильнее, чем от чего-либо
   * ещё. Пять строк, прижатые к низу, закрывают карточку целиком; три,
   * прижатые к верху, оставляют над нижним рядом дыру.
   */
  if (layout === LAYOUT.square) {
    /*
     * Самый тесный: строк три, и разложить их по стороне красивее, чем сбить
     * в кучу. Подписи вверху, кнопки посередине, полоса внизу — обложке
     * достаётся вся середина карточки.
     *
     * ВАЖНО: **полоса тут обычная, а не нитка по краю.** Ниткой она была, и
     * живая проверка это отвергла: четыре точки блёклого трека поверх кадра не
     * видно вовсе, пока трек не отыгран наполовину, — то есть полосы там будто
     * нет. Ниткой полоса остаётся только в горизонтальном пульте, где окно
     * ниже строки и другого места у неё просто нет.
     */
    if (size === SMALLEST_SIZE) {
      return (
        <div className={root}>
          {backdrop}
          <Meta player={player} />
          {buttons}
          {bar}
        </div>
      );
    }

    /*
     * Большой и средний: подписи по центру вверху, под ними управление, а низ
     * карточки отдан обложке — там же стоит и нижний ряд с плейлистами.
     * Пульт при этом читается сверху вниз одним блоком, а картинка получает
     * место, ради которого квадрат и берут.
     */
    if (size < SMALL_SQUARE) {
      return (
        <div className={root}>
          {backdrop}
          <Meta player={player} centered />
          {buttons}
          {bar}
          {sound}
        </div>
      );
    }

    // Маленький: тот же набор без громкости, но прижатый к низу — сверху ему
    // осталось бы полкарточки пустоты
    return (
      <div className={root}>
        {backdrop}
        <Meta player={player} />
        {bar}
        {buttons}
      </div>
    );
  }

  if (layout === LAYOUT.column) {
    return (
      <div className={root}>
        {backdrop}
        {art}
        <Meta player={player} centered />
        {bar}
        {buttons}
        {sound}
      </div>
    );
  }

  /*
   * Полоса. На «Большом» кнопки стоят в середине, под исполнителем и над
   * полосой перемотки: места там хватает, а справа они уводили взгляд от
   * подписей.
   *
   * На остальных размерах они возвращаются вправо, и это не только теснота: со
   * средним в той же раскладке два верхних размера отличались одной шириной,
   * то есть выбор между ними ничего не значил.
   */
  if (size === 0) {
    return (
      <div className={root}>
        {backdrop}
        {art}
        <div className="player__lane">
          <Meta player={player} />
          {buttons}
          {bar}
        </div>
      </div>
    );
  }

  /*
   * Самый тесный: полосы перемотки нет вовсе — ради этого окно там и стало на
   * строку ниже. Исключение одно, обложка фоном: у неё полоса ложится ниткой
   * по нижнему краю поверх картинки и высоты не занимает.
   */
  const thread = size === SMALLEST_SIZE && cover === COVER.back;
  const showBar = size !== SMALLEST_SIZE;

  return (
    <div className={root}>
      {backdrop}
      {art}
      <div className="player__lane">
        <Meta player={player} />
        {showBar && bar}
      </div>
      {buttons}
      {thread && edgeBar}
    </div>
  );
}

/**
 * Обложка отдельной картинкой.
 *
 * Своей высоты не назначает: в строке она равна высоте ряда, в столбике и
 * квадрате забирает то, что осталось от подписей и кнопок. Отсюда и «около
 * 80 %» у крупной — это потолок, а не размер: упрись картинка в него на
 * маленьком окне, и она вытолкнула бы кнопки за нижний край.
 */
function Cover({ url, big, link }: { url: string | null; big: boolean; link: string | null }) {
  const t = useText();
  const classes = ["player__art"];
  if (big) classes.push("player__art--big");
  // Нажатие по обложке ведёт к треку в Spotify. Не кнопка: за обложку окно
  // тянут, а сдвиг до отпускания `click` не даёт
  const open = link === null ? {} : {
    "data-link": true,
    title: t("spotify.openTrack"),
    onClick: () => openLink(link),
  };

  if (url === null) {
    return (
      <div className={classes.join(" ")}>
        <div className="player__cover player__cover--empty">
          <SpotifyMark />
        </div>
      </div>
    );
  }

  return (
    <div className={classes.join(" ")} {...open}>
      <img
        className="player__cover"
        src={url}
        alt=""
        draggable={false}
        referrerPolicy="no-referrer"
      />
    </div>
  );
}

/**
 * Обложка фоном всей карточки.
 *
 * Кладётся `position: absolute` от самой `.card`, а не от её тела: у тела есть
 * поля, и обложка с ними осталась бы картинкой в рамке вместо фона.
 *
 * ВАЖНО: **размывает `filter`, а не `backdrop-filter`.** Второй в прозрачном
 * окне сэмплировать нечего — это проверено на живых
 * окнах; здесь же размывается собственная картинка, и прозрачность окна её не
 * касается вовсе.
 *
 * Без обложки подложки нет: карточка остаётся обычной, а не темнеет впустую.
 */
function Backdrop({ url }: { url: string | null }) {
  if (url === null) return null;

  return (
    <div className="player__backdrop" aria-hidden="true">
      <img src={url} alt="" draggable={false} referrerPolicy="no-referrer" />
    </div>
  );
}

/**
 * Название с исполнителем.
 *
 * Название бежит: оно приходит извне, длину его не выбирают, и эллипсис режет
 * его ровно на том слове, ради которого на карточку и смотрят. Исполнитель
 * бежит по той же причине — «Various Artists, и ещё четверо» не редкость.
 *
 * Альбома тут больше нет ни в одной раскладке: третья строка не помещалась
 * нигде, кроме самого большого столбика, а её появление и исчезновение
 * двигало всё, что ниже.
 */
/**
 * Знак Spotify перед названием — и ссылка на трек.
 *
 * Политика разработчика Spotify (раздел II) требует у показанных обложки и
 * метаданных знака по их брендбуку и ссылки обратно (`licenses.md`). Знак
 * стоит у названия, а не на обложке: обложки нет в раскладке «фоном» и в
 * квадрате NEW, а название есть всегда. Ссылки нет (трек вспомнен из старого
 * файла) — знак остаётся подписью, не кнопкой.
 */
function TrackLink({ link }: { link: string | null }) {
  const t = useText();
  if (link === null) {
    return <span className="player__brand" aria-hidden="true"><SpotifyMark /></span>;
  }
  return (
    <button type="button" className="player__brand" data-drags title={t("spotify.openTrack")}
      aria-label={t("spotify.openTrack")} onClick={() => openLink(link)}>
      <SpotifyMark />
    </button>
  );
}

function Meta({ player, centered = false }: { player: Player; centered?: boolean }) {
  const classes = ["player__meta"];
  if (centered) classes.push("player__meta--centered");

  return (
    <div className={classes.join(" ")}>
      <div className="player__title">
        <TrackLink link={player.link} />
        <Marquee className="player__track" text={player.track} />
      </div>
      <Marquee className="player__artists" text={player.artists} />
    </div>
  );
}

/**
 * Полоса перемотки со временем по краям. Время уходит порогом по высоте.
 *
 * Вид `edge` — та же полоса ниткой по нижнему краю карточки поверх обложки:
 * высоты не занимает, времени не показывает. Им живут самые тесные размеры,
 * где на отдельную строку места нет вовсе.
 *
 * Полосу тянут мышью, а отпущенное место держится до ответа Spotify —
 * общим `useSeekDrag` (`lib/seek.ts`), тем же, что у медиа-виджета.
 */
function Progress({
  edge = false,
  elapsed,
  total,
  ratio,
  onSeek,
  subject,
}: {
  edge?: boolean;
  elapsed: number;
  total: number;
  ratio: number;
  onSeek: (share: number) => void;
  /** Чей это трек: удержание отпущенного места к другому не переходит. */
  subject: string;
}) {
  const t = useText();
  const drag = useSeekDrag(ratio, total, SEEK_CLOSE_MS, total > 0, onSeek, subject);
  const shownElapsed = drag.chosen === null ? elapsed : Math.round(drag.chosen * total);

  return (
    <div className={edge ? "player__progress player__progress--edge" : "player__progress"}>
      <div
        className="player__track-bar"
        data-no-drag
        {...drag.handlers}
        role="presentation"
        title={t("media.seek")}
      >
        <div className="player__track-fill" style={{ width: `${(drag.shown * 100).toFixed(2)}%` }} />
      </div>
      <div className="player__times tabular">
        <span>{formatTime(shownElapsed)}</span>
        <span>{formatTime(total)}</span>
      </div>
    </div>
  );
}

/**
 * Ряд кнопок. Всегда полный: перемешивание с повтором помечены `extra`, и
 * убирает их CSS по размеру окна, а не эта функция по раскладке.
 */
function Transport({
  shown,
  issue,
}: {
  shown: Player;
  issue: (action: Action, patch?: Partial<Player>) => void;
}) {
  const t = useText();
  return (
    <div className="player__controls">
      <IconButton
        label={t("spotify.shuffle")}
        active={shown.shuffle}
        extra
        onClick={() => issue({ kind: "shuffle", on: !shown.shuffle }, { shuffle: !shown.shuffle })}
      >
        <ShuffleIcon />
      </IconButton>

      <IconButton label={t("transport.previous")} onClick={() => issue({ kind: "previous" })}>
        <PreviousIcon />
      </IconButton>

      <button
        type="button"
        className="player__play"
        title={shown.isPlaying ? t("transport.pause") : t("transport.play")}
        onClick={() => issue({ kind: shown.isPlaying ? "pause" : "play" }, { isPlaying: !shown.isPlaying })}
      >
        {shown.isPlaying ? <PauseIcon /> : <PlayIcon />}
      </button>

      <IconButton label={t("transport.next")} onClick={() => issue({ kind: "next" })}>
        <NextIcon />
      </IconButton>

      <IconButton
        label={t(repeatLabel(shown.repeat))}
        active={shown.repeat !== "off"}
        extra
        onClick={() =>
          issue(
            { kind: "repeat", mode: nextRepeat(shown.repeat) },
            { repeat: nextRepeat(shown.repeat) },
          )
        }
      >
        {shown.repeat === "track" ? <RepeatOneIcon /> : <RepeatIcon />}
      </IconButton>
    </div>
  );
}

function VolumeRow({ value, onChange }: { value: number; onChange: (next: number) => void }) {
  const t = useText();
  return (
    <div className="player__volume">
      <span className="player__volume-icon">
        <VolumeIcon />
      </span>
      <Slider value={value} min={0} max={100} ariaLabel={t("spotify.volume")} onChange={onChange} />
      <span className="player__volume-value tabular">{value}%</span>
    </div>
  );
}

type IconButtonProps = {
  label: string;
  active?: boolean;
  /** Кнопка не из основных: в узкой карточке уходит. */
  extra?: boolean;
  onClick: () => void;
  children: React.ReactNode;
};

function IconButton({ label, active = false, extra = false, onClick, children }: IconButtonProps) {
  const classes = ["player__button"];
  if (active) classes.push("player__button--active");
  // Перемешивание и повтор: в узкой карточке ими жертвуют первыми
  if (extra) classes.push("player__button--extra");

  return (
    <button type="button" className={classes.join(" ")} title={label} onClick={onClick}>
      {children}
    </button>
  );
}

function repeatLabel(mode: Player["repeat"]): Key {
  if (mode === "track") return "spotify.repeatTrack";
  if (mode === "context") return "spotify.repeatList";
  return "spotify.repeatOff";
}

/** Позиция, досчитанная между опросами. */
function useTickingPosition(player: Player, receivedAt: number): number {
  const [position, setPosition] = useState(() => positionNow(player, receivedAt));

  useEffect(() => {
    setPosition(positionNow(player, receivedAt));

    // На паузе досчитывать нечего — и таймер держать незачем
    if (!player.isPlaying) return;

    const timer = window.setInterval(
      () => setPosition(positionNow(player, receivedAt)),
      TICK_MS,
    );
    return () => window.clearInterval(timer);
  }, [player, receivedAt]);

  return position;
}

/**
 * Громкость с задержкой отправки и защитой от отката.
 *
 * Ползунок ведёт себя как местный, а наружу уходит одно значение в конце
 * движения. Пока не пришло подтверждение, значение из опроса игнорируем.
 */
function useVolume(player: Player, send: (action: Action) => Promise<void>) {
  const [value, setValue] = useState<number | null>(player.volume);
  const sentAt = useRef(0);
  const timer = useRef<number | null>(null);
  const settle = useRef<number | null>(null);
  // Последняя громкость из опроса: к ней ползунок возвращается, когда
  // команда не прошла. Эффект ниже срабатывает только на её смене, а
  // отвергнутая громкость Spotify её не меняет — ползунок стоял бы на чужом
  // числе навсегда
  const reported = useRef(player.volume);
  reported.current = player.volume;

  useEffect(() => {
    if (player.volume === null) return;
    if (Date.now() - sentAt.current < VOLUME_HOLD_MS) return;
    setValue(player.volume);
  }, [player.volume]);

  useEffect(() => () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    if (settle.current !== null) window.clearTimeout(settle.current);
  }, []);

  /** Вернуть ползунок к правде, если после `stamp` его больше не трогали. */
  function resync(stamp: number) {
    if (sentAt.current !== stamp || reported.current === null) return;
    setValue(reported.current);
  }

  function change(next: number) {
    setValue(next);
    sentAt.current = Date.now();
    if (settle.current !== null) window.clearTimeout(settle.current);

    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const stamp = Date.now();
      sentAt.current = stamp;
      send({ kind: "volume", percent: next }).catch(() => resync(stamp));
      // Прошла команда, а устройство громкость не взяло — то же самое,
      // только узнаём об этом по истечении удержания
      settle.current = window.setTimeout(() => resync(stamp), VOLUME_HOLD_MS);
    }, VOLUME_DEBOUNCE_MS);
  }

  return { value, change };
}
