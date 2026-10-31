import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { useLook, useScale } from "../../lib/look";
import { useWidth } from "../../lib/size";
import { fitWave } from "./wave";
import { duration } from "../../lib/format";
import { openOutputDir } from "../../lib/output";
import { missingFor, useTools } from "../../lib/tools";
import {
  cancelConverting,
  clearFiles,
  frameAt,
  hideFrame,
  hush,
  listenFrom,
  pickFiles,
  removeFile,
  revealConverted,
  setOptions,
  startConverting,
  timecode,
  useConverter,
  waveform,
  type ConverterView,
  type Item,
  type HandleBox,
  type Options,
  type SizeChoice,
  type Target,
} from "../../lib/converter";
import { Bar } from "../../components/Bar";
import { Button } from "../../components/Button";
import { GlassCard } from "../../components/GlassCard";
import { FolderIcon, PlusIcon } from "../../components/Icons";
import { PlayIcon, StopIcon } from "../../components/Transport";
import { Toggle } from "../../components/Toggle";
import { ToolsGate } from "../../components/ToolsGate";
import "./converter.css";
import "./converter-reference.css";
import { ConverterReferenceEmpty, ConverterReferenceFooter, ConverterReferenceHeader, ConverterReferenceRow } from "./ConverterReference";
import { useText, type Text } from "../../lib/i18n";

/**
 * Конвертер: пачка файлов в MP4, GIF, MP3, PNG или JPG.
 *
 * Файлы приходят кнопкой выбора или перетаскиванием на карточку. Пачке
 * остаются только общие для всех её файлов цели, и настройки у неё одни;
 * обрезка — только у одного файла.
 *
 * Брошенные файлы ловит Rust (`file_drop.rs`), а не `onDragDropEvent`:
 * приложение повышено, и OLE-бросок из Проводника до окна не доходит. Файлы
 * приходят в список сами, а сюда — только «над карточкой несут файлы».
 */
export function ConverterWidget() {
  const t = useText();
  const tools = useTools();
  const view = useConverter();
  const dragging = useDropHover();
  const reference = useLook().effectiveDesign === "reference";
  const ready = missingFor(tools, "converter").length === 0 && !tools.probing;

  return (
    <div className={reference ? "widget widget--converter-reference" : "widget"}>
      <GlassCard accent="neutral">
        <div className={reference ? "conv conv--reference" : "conv"}>
          {!ready ? (
            <ToolsGate module="converter" />
          ) : view.items.length === 0 ? (
            <Empty reference={reference} />
          ) : (
            <Batch view={view} reference={reference} />
          )}
          {/* Поверх карточки, а не строкой: место под ней не нужно, пока
              файл не несут */}
          {dragging && ready && !view.running && <div className="conv__drop">{t("converter.dropToAdd")}</div>}
        </div>
      </GlassCard>
    </div>
  );
}

/**
 * Над карточкой несут файлы. Говорит об этом Rust: сам webview о чужом
 * переносе не знает — до него OLE у повышенного окна не доходит.
 *
 * Вне Tauri, в браузере, где карточку меряют, подписка просто не встаёт.
 */
function useDropHover(): boolean {
  const [over, setOver] = useState(false);

  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    listen<boolean>("converter-drop", (event) => setOver(event.payload))
      .then((off) => {
        // Отписка обязана пережить повторный монтаж StrictMode: снимаем ровно
        // то, что поставили, когда оно наконец поставилось
        if (alive) stop = off;
        else off();
      })
      .catch(() => {});
    return () => {
      alive = false;
      stop?.();
    };
  }, []);

  return over;
}

function Empty({ reference }: { reference: boolean }) {
  const t = useText();
  if (reference) return <ConverterReferenceEmpty />;
  return (
    <div className="conv__empty">
      <p className="conv__lead">{t("converter.dropHere")}</p>
      <Button onClick={() => void pickFiles()}>{t("converter.choose")}</Button>
      {/* Названия видов — из каталога, как у NEW: это не подписи показаний,
          и английскими на любом языке им стоять незачем. Форматы — имена */}
      <p className="conv__hint">
        {t("converter.state.video")} → MP4, GIF, MP3 · {t("converter.state.audio")} → MP3 · {t("converter.state.image")} → PNG, JPG
      </p>
    </div>
  );
}

function Batch({ view, reference }: { view: ConverterView; reference: boolean }) {
  const t = useText();
  const { items, options, targets, target, running } = view;
  const canStart =
    !running &&
    target !== null &&
    view.trim.error === null &&
    items.some((item) => item.status === "ready");

  function change(patch: Partial<Options>) {
    void setOptions({ ...options, ...patch });
  }

  const list = (
    <ul key="list" className="conv__list">
        {items.map((item) => (
          <Row key={item.id} item={item} running={running} reference={reference} />
        ))}
    </ul>
  );
  const note = view.mixed ? <p key="note" className="conv__note">{t("converter.mixed")}</p> : null;
  const recipe = targets.length > 0 ? (
    <div key="options" className="conv__options">
          {reference && running && <p className="conv-ref__next">{t("converter.nextRunOptions")}</p>}
          <Choice label={t("converter.to")}>
            {targets.map((each) => (
              <Toggle
                key={each}
                label={each.toUpperCase()}
                active={target === each}
                onChange={() => change({ target: each })}
              />
            ))}
          </Choice>
          {target !== null && (
            <TargetOptions target={target} options={options} sizes={view.sizes} change={change} />
          )}
          {target !== null && view.trim.enabled && target !== "png" && target !== "jpg" && (
            <TrimFields view={view} change={change} />
          )}
    </div>
  ) : null;
  const footer = reference ? (
    <ConverterReferenceFooter key="footer" view={view} canStart={canStart} />
  ) : (
    <div key="footer" className="conv__foot">
        {running ? (
          <>
            <div className="busy-bar conv__overall">
              <Bar fill={view.overall} thickness="thin" />
            </div>
            <Button variant="outline" onClick={() => void cancelConverting()}>
              {t("common.cancel")}
            </Button>
          </>
        ) : (
          <>
            <Button variant="outline" icon={<PlusIcon />} label={t("converter.add")} onClick={() => void pickFiles()} />
            <Button variant="outline" icon="✕" label={t("converter.clearList")} onClick={() => void clearFiles()} />
            <Button
              variant="outline"
              icon={<FolderIcon />}
              label={t("converter.openFolder")}
              onClick={() => void openOutputDir("converter").catch(() => {})}
            />
            <span className="conv__spacer" />
            <Button onClick={() => void startConverting().catch(() => {})} disabled={!canStart}>
              {t("converter.convert")}
            </Button>
          </>
        )}
    </div>
  );
  const sections = reference
    ? [<ConverterReferenceHeader key="header" view={view} />, note, recipe, list, footer]
    : [list, note, recipe, footer];
  return <>{sections}</>;
}

function Row({ item, running, reference }: { item: Item; running: boolean; reference: boolean }) {
  const t = useText();
  if (reference) return <ConverterReferenceRow item={item} running={running} state={item.status === "probing" ? t("converter.state.probing") : stateText(t, item)} />;
  return (
    <li className="conv__item" data-status={item.status}>
      <span className="conv__name" title={item.name}>
        {item.name}
      </span>
      <span className="conv__state tabular" title={item.reason ?? item.output ?? undefined}>
        {stateText(t, item)}
      </span>
      {item.status === "done" ? (
        <button
          type="button"
          className="conv__action"
          title={t("converter.reveal", { name: item.output ?? t("converter.theFile") })}
          onClick={() => void revealConverted(item.id)}
        >
          <FolderIcon />
        </button>
      ) : (
        <button
          type="button"
          className="conv__action conv__action--remove"
          title={t("common.remove")}
          disabled={running}
          onClick={() => void removeFile(item.id)}
        >
          ✕
        </button>
      )}
      {item.status === "running" && (
        <div className="busy-bar conv__bar">
          <Bar fill={item.progress} thickness="thin" />
        </div>
      )}
    </li>
  );
}

function stateText(t: Text, item: Item): string {
  switch (item.status) {
    case "probing":
      return "…";
    case "ready":
      if (item.kind === "image") return t("converter.state.image");
      return `${t(item.kind === "audio" ? "converter.state.audio" : "converter.state.video")} · ${duration(item.duration)}`;
    case "unsupported":
      return t("converter.state.unsupported");
    case "running":
      return item.progress === null ? t("converter.state.working") : `${Math.round(item.progress * 100)}%`;
    case "done":
      return t("converter.state.done");
    case "failed":
      return t("converter.state.failed");
    case "cancelled":
      return t("tools.failure.cancelled");
  }
}

function Choice({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="conv__choice">
      <span className="conv__label caps">{label}</span>
      <div className="conv__chips">{children}</div>
    </div>
  );
}

type ChangeProps = { options: Options; change: (patch: Partial<Options>) => void };

/**
 * Ряд потолков — только тех, что хоть что-то уменьшат. Ряд и подсвеченный
 * потолок решает Rust: увеличения нет, и «1080p» у ролика 720p ничего бы не
 * сделал. Сам выбор при этом не переписывается — вернётся с файлом побольше.
 */
function SizeChips({
  label,
  sizes,
  name,
  pick,
}: {
  label: string;
  sizes: SizeChoice;
  name: (size: number) => string;
  pick: (size: number) => void;
}) {
  const t = useText();
  return (
    <Choice label={label}>
      {sizes.offered.map((size) => (
        <Toggle
          key={size}
          label={size === 0 ? t("converter.original") : name(size)}
          short={size === 0 ? t("converter.originalShort") : undefined}
          active={sizes.chosen === size}
          onChange={() => pick(size)}
        />
      ))}
    </Choice>
  );
}

function TargetOptions({
  target,
  options,
  sizes,
  change,
}: ChangeProps & { target: Target; sizes: ConverterView["sizes"] }) {
  const t = useText();
  switch (target) {
    case "mp4":
      return (
        <>
          <Choice label={t("converter.quality")}>
            {(["high", "balanced", "small"] as const).map((quality) => (
              <Toggle
                key={quality}
                label={t(`converter.quality.${quality}`)}
                short={t(`converter.quality.${quality}Short`)}
                active={options.quality === quality}
                onChange={() => change({ quality })}
              />
            ))}
          </Choice>
          <SizeChips
            label={t("converter.size")}
            sizes={sizes.video}
            name={(size) => `${size}p`}
            pick={(videoSize) => change({ videoSize })}
          />
        </>
      );
    case "gif":
      return (
        <>
          <SizeChips
            label={t("converter.width")}
            sizes={sizes.gif}
            name={String}
            pick={(gifSize) => change({ gifSize })}
          />
          <Choice label="FPS">
            {[10, 15, 24].map((fps) => (
              <Toggle
                key={fps}
                label={`${fps}`}
                short={`${fps} fps`}
                active={options.gifFps === fps}
                onChange={() => change({ gifFps: fps })}
              />
            ))}
          </Choice>
        </>
      );
    case "mp3":
      return (
        <Choice label={t("converter.bitrate")}>
          {[320, 192, 128].map((bitrate) => (
            <Toggle
              key={bitrate}
              label={`${bitrate} kbps`}
              short={`${bitrate}k`}
              active={options.bitrate === bitrate}
              onChange={() => change({ bitrate })}
            />
          ))}
        </Choice>
      );
    case "png":
    case "jpg":
      // У картинок — только уменьшение, без сжатия: так решил пользователь
      return (
        <SizeChips
          label={t("converter.size")}
          sizes={sizes.image}
          name={String}
          pick={(imageSize) => change({ imageSize })}
        />
      );
  }
}

/**
 * Обрезка: оставить отрезок «от–до». Поля держат черновик у себя, а в Rust
 * уходит каждая правка — там же она и проверяется против длины файла.
 *
 * ВАЖНО: **ответ Rust не перезаписывает поле, пока в нём печатают.** Эхо
 * правки приходит событием чуть позже, и запоздавшее эхо прошлой буквы съело
 * бы только что набранную. Из Rust поле берёт значение, только когда оно не в
 * фокусе, — например, когда пачку очистили.
 */
function TrimFields({ view, change }: { view: ConverterView; change: ChangeProps["change"] }) {
  const t = useText();
  const [from, setFrom] = useState(view.options.from);
  const [to, setTo] = useState(view.options.to);
  const editing = useRef(false);

  useEffect(() => {
    if (!editing.current) setFrom(view.options.from);
  }, [view.options.from]);
  useEffect(() => {
    if (!editing.current) setTo(view.options.to);
  }, [view.options.to]);

  const field = (value: string, placeholder: string, label: string, apply: (next: string) => void) => (
    <input
      type="text"
      className="input conv__time tabular"
      value={value}
      placeholder={placeholder}
      spellCheck={false}
      autoComplete="off"
      data-no-drag
      aria-label={label}
      onFocus={() => {
        editing.current = true;
      }}
      onBlur={() => {
        editing.current = false;
        // Эхо могло прийти в фокусе или вовсе не изменить прежнее значение
        setFrom(view.options.from);
        setTo(view.options.to);
      }}
      onChange={(event) => apply(event.target.value)}
    />
  );

  const length = view.trim.duration;
  const preview = usePreview(view);

  /*
   * Полоса пишет в те же поля. Края — пустые поля: «с начала» и «до конца»
   * так и читаются, а число у края промахивалось бы на долю секунды.
   */
  function slide(nextFrom: number, nextTo: number, commit: boolean) {
    if (length === null) return;
    const tenths = trimStep(length) < 1;
    const fromText = nextFrom <= 0 ? "" : timecode(nextFrom, tenths);
    const toText = nextTo >= length ? "" : timecode(nextTo, tenths);
    setFrom(fromText);
    setTo(toText);
    if (commit) change({ from: fromText, to: toText });
  }

  const start = view.trim.from ?? 0;
  const end = view.trim.to ?? length ?? 0;

  return (
    // Обёртка ничего не значит в полной раскладке (`display: contents`), а в
    // тесной собирает поля и полосу в одну строку
    <div className="conv__cut">
      <Choice label={t("converter.trim")}>
        {field(from, "0:00", t("converter.keepFrom"), (next) => {
          setFrom(next);
          change({ from: next, to });
        })}
        <span className="conv__dash">–</span>
        {field(to, duration(length), t("converter.keepUntil"), (next) => {
          setTo(next);
          change({ from, to: next });
        })}
        {view.trim.kind === "audio" && (
          <button
            type="button"
            className="conv__action conv__listen"
            title={preview.playing ? t("converter.stop") : t("converter.listen")}
            onClick={() => (preview.playing ? preview.stop() : preview.play(start, end, false))}
          >
            {preview.playing ? <StopIcon /> : <PlayIcon />}
          </button>
        )}
        {view.trim.error !== null && <span className="conv__error">{t(`converter.trim.${view.trim.error}`)}</span>}
      </Choice>
      {length !== null && length > 0 && (
        <TrimBar
          length={length}
          from={start}
          to={end}
          wave={preview.wave}
          playhead={preview.playhead}
          onSlide={slide}
          onGrab={preview.forget}
          onRelease={preview.released}
          onHover={preview.hover}
        />
      )}
    </div>
  );
}

/** Шаг полосы: у коротких файлов — десятые доли секунды, у длинных — секунды. */
function trimStep(length: number): number {
  return length <= 60 ? 0.1 : 1;
}

type Edge = "from" | "to";

/** Сколько кадр держится над ручкой, когда мышь ушла с полосы. */
const FRAME_LINGER_MS = 1500;

/**
 * Превью обрезки: кадр над ручкой у видео, волна и прослушивание у звука.
 * Всё это достаёт Rust тем же ffmpeg (`converter/preview.rs`); здесь — когда
 * просить и сколько держать.
 *
 * Кадр просят на отпускании ручки, а не на каждом сдвиге: у 4K он идёт почти
 * секунду. Встаёт он своим окном над ручкой, втрое крупнее первой редакции и
 * хоть за границей виджета, — просьба пользователя. Держится, пока мышь над
 * полосой, и ещё полторы секунды после.
 * Звук заигрывает на отпускании: левая ручка — с неё вперёд, правая —
 * последние секунды перед ней, то есть то, чем отрезок кончится.
 */
function usePreview(view: ConverterView) {
  const { kind, subject, duration: length } = view.trim;
  const [wave, setWave] = useState<number[] | null>(null);
  const [playing, setPlaying] = useState<{ start: number; length: number; since: number } | null>(null);
  const [now, setNow] = useState(0);

  const hovering = useRef(false);
  const hide = useRef<number | undefined>(undefined);
  // Запоздавший кадр прежнего отпускания не должен перебить новый
  const ticket = useRef(0);
  const playTicket = useRef(0);

  // Волна — одна на файл; сменился файл — и кадр, и звук тоже чужие
  useEffect(() => {
    ticket.current += 1;
    window.clearTimeout(hide.current);
    setWave(null);
    void hideFrame().catch(() => {});
    if (kind !== "audio" || subject === null) return;
    let alive = true;
    waveform()
      .then((bars) => {
        if (alive) setWave(bars);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [kind, subject]);

  // Звук не переживает ни смену файла, ни уход карточки
  useEffect(
    () => () => {
      playTicket.current += 1;
      setPlaying(null);
      void hush().catch(() => {});
    },
    [kind, subject],
  );

  // Бегунок идёт по часам: позицию у `PlaySound` не спросить
  useEffect(() => {
    if (playing === null) return;
    const tick = window.setInterval(() => {
      const passed = (performance.now() - playing.since) / 1000;
      if (passed >= playing.length) setPlaying(null);
      else setNow(passed);
    }, 50);
    return () => window.clearInterval(tick);
  }, [playing]);

  // Карточку закрыли — окно кадра над ней оставаться не должно
  useEffect(
    () => () => {
      ticket.current += 1;
      window.clearTimeout(hide.current);
      void hideFrame().catch(() => {});
    },
    [],
  );

  function linger() {
    window.clearTimeout(hide.current);
    hide.current = window.setTimeout(() => void hideFrame().catch(() => {}), FRAME_LINGER_MS);
  }

  function play(from: number, until: number, tail: boolean) {
    const mine = ++playTicket.current;
    listenFrom(from, until, tail)
      .then((started) => {
        // Ответ прежнего куска после остановки или новой просьбы уже чужой
        if (mine !== playTicket.current) return;
        setNow(0);
        setPlaying({ ...started, since: performance.now() });
      })
      .catch(() => {});
  }

  return {
    wave,
    playing: playing !== null,
    playhead: playing === null ? null : playing.start + Math.min(now, playing.length),
    play,
    stop() {
      playTicket.current += 1;
      setPlaying(null);
      void hush().catch(() => {});
    },
    /** Ручку взяли: прежний кадр уже про другое место. */
    forget() {
      ticket.current += 1;
      window.clearTimeout(hide.current);
      void hideFrame().catch(() => {});
    },
    released(edge: Edge, from: number, until: number, handle: HandleBox | null) {
      if (kind === "audio") {
        play(from, until, edge === "to");
        return;
      }
      if (kind !== "video" || length === null || handle === null) return;
      const mine = ++ticket.current;
      frameAt(edge === "from" ? from : until, handle)
        .then(() => {
          // Rust проверяет билет до показа: запоздавший ответ не должен
          // ни продлевать прежний кадр, ни прятать уже попрошенный новый
          if (mine !== ticket.current) return;
          window.clearTimeout(hide.current);
          // Отпустили стрелками, а мышь где-то ещё — кадр всё равно покажем
          if (!hovering.current) linger();
        })
        .catch(() => {});
    },
    hover(over: boolean) {
      hovering.current = over;
      if (over) window.clearTimeout(hide.current);
      else linger();
    },
  };
}

/**
 * Полоса отрезка под полями: дорожка — весь файл, светлое — то, что
 * останется, ручки — его края. Ручку тянут, а нажатие в дорожку переносит
 * туда ближнюю ручку. Стрелки у ручки в фокусе двигают её на секунду.
 *
 * У звука дорожка — волна, и светлее та её часть, что останется; у видео над
 * отпущенной ручкой всплывает кадр из этого места (своим окном, его ставит
 * Rust). Кадров вдоль всей полосы нет: пришлось бы вынимать их десятком из
 * каждого файла.
 *
 * ВАЖНО: **в Rust уходит только отпускание.** Движение мыши — десятки событий
 * в секунду, и каждое разослало бы снимок во все окна. Пока тянут, ручки и
 * поля живут здесь; отпущенные ручки держат место, пока эхо из Rust не
 * принесёт новые числа, — иначе на кадр отскочили бы назад.
 */
function TrimBar({
  length,
  from,
  to,
  wave,
  playhead,
  onSlide,
  onGrab,
  onRelease,
  onHover,
}: {
  length: number;
  from: number;
  to: number;
  wave: number[] | null;
  playhead: number | null;
  onSlide: (from: number, to: number, commit: boolean) => void;
  onGrab: () => void;
  onRelease: (edge: Edge, from: number, to: number, handle: HandleBox | null) => void;
  onHover: (over: boolean) => void;
}) {
  const t = useText();
  const track = useRef<HTMLDivElement>(null);
  const width = useWidth(track);
  const scale = useScale();
  const bars = wave === null ? [] : fitWave(wave, width, scale);
  const grabbed = useRef<Edge | null>(null);
  const [held, setHeld] = useState<{ from: number; to: number } | null>(null);

  // Эхо пришло — ручки снова стоят по полям
  useEffect(() => setHeld(null), [from, to, length]);

  const step = trimStep(length);
  const shown = held ?? { from: Math.min(from, length), to: Math.min(Math.max(to, 0), length) };
  // Последнее положение — в ref: события мыши приходят чаще перерисовок
  const current = useRef(shown);
  current.current = shown;

  function at(clientX: number): number {
    const rect = track.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 0;
    const raw = Math.min(length, Math.max(0, ((clientX - rect.left) / rect.width) * length));
    // У правого края — ровно край: иначе округление до шага оставляло бы
    // «до конца» на полсекунды раньше конца
    if (raw > length - step / 2) return length;
    return Math.round(raw / step) * step;
  }

  function move(edge: Edge, value: number, commit: boolean) {
    const { from: a, to: b } = current.current;
    // Ручки не заходят друг за друга: пустого отрезка не бывает
    const next =
      edge === "from"
        ? { from: Math.max(0, Math.min(value, b - step)), to: b }
        : { from: a, to: Math.min(length, Math.max(value, a + step)) };
    current.current = next;
    setHeld(next);
    onSlide(next.from, next.to, commit);
    if (commit) onRelease(edge, next.from, next.to, box(edge));
  }

  /** Где ручка в окне — над ней Rust поставит кадр. */
  function box(edge: Edge): HandleBox | null {
    const handle = track.current?.querySelectorAll(".conv__trim-handle")[edge === "from" ? 0 : 1];
    if (!(handle instanceof HTMLElement)) return null;
    const rect = handle.getBoundingClientRect();
    const rail = track.current!.getBoundingClientRect();
    // После стрелки React ещё не переставил ручку: её центр считаем из
    // нового значения, а высоту берём у той же ручки в нынешней раскладке
    return { x: rail.left + (current.current[edge] / length) * rail.width, top: rect.top, bottom: rect.bottom };
  }

  function down(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    const value = at(event.clientX);
    const { from: a, to: b } = current.current;
    const edge: Edge = Math.abs(value - a) < Math.abs(value - b) ? "from" : "to";
    grabbed.current = edge;
    event.currentTarget.setPointerCapture(event.pointerId);
    onGrab();
    move(edge, value, false);
  }

  function drag(event: PointerEvent<HTMLDivElement>) {
    if (grabbed.current !== null) move(grabbed.current, at(event.clientX), false);
  }

  function release() {
    const edge = grabbed.current;
    if (edge === null) return;
    const { from: a, to: b } = current.current;
    grabbed.current = null;
    onSlide(a, b, true);
    onRelease(edge, a, b, box(edge));
  }

  function key(edge: Edge, event: KeyboardEvent<HTMLSpanElement>) {
    const nudge = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[event.key];
    if (nudge === undefined) return;
    event.preventDefault();
    onGrab();
    const base = current.current[edge];
    move(edge, Math.min(length, Math.max(0, base + nudge * Math.max(step, 1))), true);
  }

  const left = (shown.from / length) * 100;
  const right = (shown.to / length) * 100;

  const handle = (edge: Edge, percent: number, label: string) => (
    <span
      role="slider"
      tabIndex={0}
      className="conv__trim-handle"
      style={{ left: `${percent}%` }}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={length}
      aria-valuenow={shown[edge]}
      aria-valuetext={timecode(shown[edge], step < 1)}
      onKeyDown={(event) => key(edge, event)}
    />
  );

  return (
    <div
      ref={track}
      className={wave === null ? "conv__trim" : "conv__trim conv__trim--wave"}
      data-no-drag
      onPointerDown={down}
      onPointerMove={drag}
      onPointerUp={release}
      onPointerCancel={release}
      onPointerEnter={() => onHover(true)}
      onPointerLeave={() => onHover(false)}
    >
      {wave === null ? (
        <div className="conv__trim-kept" style={{ left: `${left}%`, width: `${right - left}%` }} />
      ) : (
        <>
          <Wave bars={bars} />
          <Wave bars={bars} clip={`inset(0 ${100 - right}% 0 ${left}%)`} />
        </>
      )}
      {playhead !== null && (
        <span className="conv__trim-playhead" style={{ left: `${(playhead / length) * 100}%` }} />
      )}
      {handle("from", left, t("converter.keepFrom"))}
      {handle("to", right, t("converter.keepUntil"))}
    </div>
  );
}

/**
 * Волна рядом `<span>` с высотой в процентах, как история столбиками в
 * `Columns`: тянется вместе с полосой сама. Светлая копия поверх тусклой
 * обрезана по оставляемому отрезку — так видно, что уйдёт.
 */
function Wave({ bars, clip }: { bars: number[]; clip?: string }) {
  return (
    <div
      className={clip === undefined ? "conv__wave" : "conv__wave conv__wave--kept"}
      style={clip === undefined ? undefined : { clipPath: clip }}
      aria-hidden="true"
    >
      {bars.map((bar, index) => (
        <span key={index} style={{ height: `${Math.max(bar * 100, 6)}%` }} />
      ))}
    </div>
  );
}
