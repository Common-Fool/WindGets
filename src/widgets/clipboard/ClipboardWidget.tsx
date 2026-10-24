import { useEffect, useRef, useState } from "react";
import { useText } from "../../lib/i18n";
import {
  clipboardThumb, control, controlConfirmed, useClipboard,
  type Action, type ClearTarget, type ControlFailure, type Entry, type ImageEntry,
} from "../../lib/clipboard";
import { useLook } from "../../lib/look";
import { CloseIcon, DownloadIcon, EraseIcon, FailedIcon, PinIcon } from "../../components/Icons";
import { GlassCard } from "../../components/GlassCard";
import { Button } from "../../components/Button";
import "./clipboard.css";
import "./clipboard-design.css";

/** Claude кратко опережает снимок Rust при возврате текста в буфер. */
const HOLD_MS = 400;
const NO_IMAGES: ImageEntry[] = [];

type Thumbnail = { source: string | null };

/** Кэш живёт над вкладками: переключение не запрашивает пиксели повторно. */
function useThumbnails(images: ImageEntry[]) {
  const cache = useRef(new Map<number, Thumbnail>());
  const mounted = useRef(false);
  const [, refresh] = useState(0);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    const ids = new Set(images.map((image) => image.id));
    for (const id of cache.current.keys()) {
      if (!ids.has(id)) cache.current.delete(id);
    }
    for (const { id } of images) {
      if (cache.current.has(id)) continue;
      const thumbnail: Thumbnail = { source: null };
      // Резервируем id до IPC: повторный снимок не запускает второй запрос.
      cache.current.set(id, thumbnail);
      void clipboardThumb(id).then((source) => {
        if (!mounted.current || cache.current.get(id) !== thumbnail) return;
        thumbnail.source = source;
        refresh((value) => value + 1);
      });
    }
  }, [images]);
  return cache.current;
}

function failureLabel(t: ReturnType<typeof useText>, failure: ControlFailure, pins: number) {
  if (failure === "pinsFull") return t("clipboard.pinsFull", { count: pins });
  if (failure === "saveFailed") return t("clipboard.saveFailed");
  if (failure === "clearFailed") return t("clipboard.clearFailed");
  return t("clipboard.copyFailed");
}

function Tabs({ tab, onChange }: {
  tab: ClearTarget;
  onChange: (tab: ClearTarget) => void;
}) {
  const t = useText();
  return (
    <div className="clip-tabs" data-no-drag>
      <button type="button" aria-pressed={tab === "text"} onClick={() => onChange("text")}>
        {t("clipboard.tabText")}
      </button>
      <button type="button" aria-pressed={tab === "images"} onClick={() => onChange("images")}>
        {t("clipboard.tabImages")}
      </button>
    </div>
  );
}

function ImageGrid({ images, thumbnails, onAction }: {
  images: ImageEntry[];
  thumbnails: Map<number, Thumbnail>;
  onAction: (action: Action) => void;
}) {
  const t = useText();
  return (
    <ul className="clip-images" aria-label={t("clipboard.tabImages")}>
      {images.map((image) => {
        const source = thumbnails.get(image.id)?.source;
        const dimensions = image.width + "×" + image.height;
        return (
          <li key={image.id} className="clip-images__tile" data-current={image.current}>
            <button
              type="button"
              className="clip-images__copy"
              title={dimensions}
              aria-label={t("clipboard.tabImages") + " " + dimensions}
              aria-current={image.current ? "true" : undefined}
              onClick={() => onAction({ kind: "copyImage", id: image.id })}
            >
              {source && <img src={source} alt="" draggable={false} />}
            </button>
            <div className="clip-images__tools">
              <button type="button" className="clip-images__save"
                title={t("clipboard.saveImage")} aria-label={t("clipboard.saveImage")}
                onClick={() => onAction({ kind: "saveImage", id: image.id })}>
                <DownloadIcon />
              </button>
              <button type="button" className="clip-images__drop"
                title={t("common.remove")} aria-label={t("common.remove")}
                onClick={() => onAction({ kind: "removeImage", id: image.id })}>
                <CloseIcon />
              </button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

type RowProps = {
  entry: Entry;
  current: boolean;
  onCopy: () => void;
  onDrop: () => void;
  onPin: () => void;
};

function PinButton({ entry, className, onClick }: {
  entry: Entry;
  className: string;
  onClick: () => void;
}) {
  const t = useText();
  const label = t(entry.pinned ? "clipboard.unpin" : "clipboard.pin");
  return (
    <button type="button" className={className} data-pinned={entry.pinned}
      title={label} aria-label={label} aria-pressed={entry.pinned} onClick={onClick}>
      <PinIcon />
    </button>
  );
}

function Row({ entry, current, onCopy, onDrop, onPin }: RowProps) {
  const t = useText();
  return (
    <li className="clip__item" data-current={current}>
      {/* Действия соседние: вложенные кнопки всплывали бы в копирование. */}
      <button
        type="button"
        className="clip__text"
        title={entry.preview + "\n" + t.count("clipboard.chars", entry.chars)}
        onClick={onCopy}
      >
        {entry.preview}
      </button>
      <PinButton entry={entry} className="clip__pin" onClick={onPin} />
      <button type="button" className="clip__drop" title={t("common.remove")}
        aria-label={t("common.remove")} onClick={onDrop}>
        <CloseIcon />
      </button>
    </li>
  );
}

function AppClipboard() {
  const t = useText();
  const { entries, capacity, images = NO_IMAGES, imageCapacity = 0 } = useClipboard();
  const [tab, setTab] = useState<ClearTarget>("text");
  const thumbnails = useThumbnails(images);
  const [picked, setPicked] = useState<number | undefined>(undefined);
  const [failure, setFailure] = useState<ControlFailure | undefined>(undefined);
  const [clearing, setClearing] = useState(false);
  const request = useRef(0);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => {
    window.clearTimeout(timer.current);
    request.current += 1;
  }, []);

  function reset() {
    request.current += 1;
    setFailure(undefined);
    setClearing(false);
  }

  function copy(id: number) {
    reset();
    setPicked(id);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setPicked(undefined), HOLD_MS);
    control({ kind: "copy", id });
  }

  async function act(action: Action) {
    const ticket = ++request.current;
    setFailure(undefined);
    setClearing(action.kind === "clear");
    const result = await controlConfirmed(action);
    if (ticket !== request.current) return;
    setClearing(false);
    if (!result.ok) setFailure(action.kind === "clear" ? "clearFailed" :
      action.kind === "saveImage" ? "saveFailed" : result.reason);
  }

  function selectTab(next: ClearTarget) {
    if (next === tab) return;
    reset();
    setTab(next);
  }

  const current = picked ?? entries.find((entry) => entry.current)?.id;
  const count = tab === "text" ? entries.length : images.length;
  const limit = tab === "text" ? capacity : imageCapacity;
  const historyCount = tab === "text" ? entries.filter((entry) => !entry.pinned).length : count;
  const failureText = failure && failureLabel(t, failure, entries.filter((entry) => entry.pinned).length);

  return (
    <div className="widget">
      <GlassCard accent="neutral">
        <div className="clip">
          {tab === "images" ? (
            images.length === 0 ? <p className="clip__empty">{t("clipboard.imagesEmpty")}</p> :
              <ImageGrid images={images} thumbnails={thumbnails} onAction={(action) => { void act(action); }} />
          ) : entries.length === 0 ? (
            /* История пуста, но в системном буфере может быть прежний текст. */
            <p className="clip__empty">{t("clipboard.empty")}</p>
          ) : (
            <ul className="clip__list">
              {entries.map((entry) => (
                <Row
                  key={entry.id}
                  entry={entry}
                  current={entry.id === current}
                  onCopy={() => copy(entry.id)}
                  onDrop={() => { reset(); control({ kind: "remove", id: entry.id }); }}
                  onPin={() => { void act({ kind: "pin", id: entry.id, pinned: !entry.pinned }); }}
                />
              ))}
            </ul>
          )}
          <div className="clip__foot">
            <Tabs tab={tab} onChange={selectTab} />
            <span className="clip__count tabular" data-error={failure || undefined}
              role={failure ? "alert" : undefined} title={failureText || undefined}>
              {failureText || t("clipboard.count", { count: historyCount, capacity: limit })}
            </span>
            <Button
              variant="outline"
              icon={<EraseIcon />}
              label={t(tab === "text" ? "clipboard.clearHint" : "clipboard.clearImagesHint")}
              disabled={clearing || (count === 0 && failure !== "clearFailed")}
              onClick={() => { void act({ kind: "clear", what: tab }); }}
            />
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

/** GPT сохраняет многострочный предпросмотр; справа число знаков и действия. */
function ReferenceRow({ entry, current, pending, failed, onCopy, onDrop, onPin }: RowProps & {
  pending: boolean;
  failed: boolean;
}) {
  const t = useText();
  const preview = entry.formattedPreview?.trim() ? entry.formattedPreview : entry.preview;
  const title = preview + "\n" + t.count("clipboard.chars", entry.chars);
  return (
    <li className="clip-ref__item" data-current={current} data-failed={failed}>
      <button type="button" className="clip-ref__text" title={title} aria-busy={pending}
        aria-current={current ? "true" : undefined} onClick={onCopy}>
        <span className="clip-ref__preview">{preview}</span>
      </button>
      {/* Действия сверху, число знаков под ними: один блок у правого края */}
      <span className="clip-ref__side">
        <span className="clip-ref__tools">
          <PinButton entry={entry} className="clip-ref__pin" onClick={onPin} />
          <button type="button" className="clip-ref__drop" title={t("common.remove")}
            aria-label={t("common.remove")} onClick={onDrop}>
            <CloseIcon />
          </button>
        </span>
        <span className="clip-ref__meta tabular">
          {t.count("clipboard.charsShort", entry.chars)}
        </span>
      </span>
    </li>
  );
}

function ReferenceClipboard() {
  const t = useText();
  const { entries, capacity, images = NO_IMAGES, imageCapacity = 0 } = useClipboard();
  const [tab, setTab] = useState<ClearTarget>("text");
  const thumbnails = useThumbnails(images);
  const [pending, setPending] = useState<number | undefined>(undefined);
  const [failed, setFailed] = useState<number | undefined>(undefined);
  const [failure, setFailure] = useState<ControlFailure | undefined>(undefined);
  const [clearing, setClearing] = useState(false);
  const request = useRef(0);
  useEffect(() => () => { request.current += 1; }, []);

  function reset() {
    request.current += 1;
    setPending(undefined);
    setFailed(undefined);
    setFailure(undefined);
    setClearing(false);
  }

  async function act(action: Action) {
    const ticket = ++request.current;
    setPending(action.kind === "copy" || action.kind === "pin" ? action.id : undefined);
    setFailed(undefined);
    setFailure(undefined);
    setClearing(action.kind === "clear");
    const result = await controlConfirmed(action);
    // Ни старый ответ, ни ответ разобранного дизайна не меняют карточку.
    if (ticket !== request.current) return;
    setPending(undefined);
    setClearing(false);
    if (!result.ok) {
      setFailure(action.kind === "clear" ? "clearFailed" :
        action.kind === "saveImage" ? "saveFailed" : result.reason);
      if (action.kind === "copy" || action.kind === "pin") setFailed(action.id);
    }
    // Текущие строки и картинки ведёт только снимок Rust.
  }

  function remove(id: number) {
    reset();
    control({ kind: "remove", id });
  }

  function selectTab(next: ClearTarget) {
    if (next === tab) return;
    reset();
    setTab(next);
  }

  const count = tab === "text" ? entries.length : images.length;
  const limit = tab === "text" ? capacity : imageCapacity;
  const historyCount = tab === "text" ? entries.filter((entry) => !entry.pinned).length : count;
  const failureText = failure && failureLabel(t, failure, entries.filter((entry) => entry.pinned).length);

  return (
    <div className="widget clip-widget--reference">
      <GlassCard accent="neutral" flush>
        <div className="clip-ref">
          <div className="clip-ref__viewport">
            {tab === "images" ? (
              images.length === 0 ? <p className="clip-ref__empty">{t("clipboard.imagesEmpty")}</p> :
                <ImageGrid images={images} thumbnails={thumbnails} onAction={(action) => { void act(action); }} />
            ) : entries.length === 0 ? (
              <p className="clip-ref__empty">{t("clipboard.empty")}</p>
            ) : (
              <ul
                className="clip-ref__list"
                tabIndex={0}
                aria-label={t("widget.clipboard")}
                onMouseDown={(event) => {
                  // Бегунок держит мышь, пустота и подписи двигают окно.
                  const list = event.currentTarget;
                  const scrollbar = list.offsetWidth - list.clientWidth;
                  if (scrollbar > 0 && event.clientX >= list.getBoundingClientRect().right - scrollbar) {
                    event.stopPropagation();
                  }
                }}
              >
                {entries.map((entry) => (
                  <ReferenceRow
                    key={entry.id}
                    entry={entry}
                    current={entry.current}
                    pending={entry.id === pending}
                    failed={entry.id === failed}
                    onCopy={() => { void act({ kind: "copy", id: entry.id }); }}
                    onDrop={() => remove(entry.id)}
                    onPin={() => { void act({ kind: "pin", id: entry.id, pinned: !entry.pinned }); }}
                  />
                ))}
              </ul>
            )}
          </div>
          <div className="clip-ref__foot">
            {failureText && (
              <div className="clip-ref__error" role="status" title={failureText}>
                <FailedIcon />
                <span>{failureText}</span>
              </div>
            )}
            <div className="clip-ref__actions">
              <Tabs tab={tab} onChange={selectTab} />
              <span className="clip-ref__count tabular">
                {t("clipboard.count", { count: historyCount, capacity: limit })}
              </span>
              <button type="button" className="clip-ref__clear"
                title={t(tab === "text" ? "clipboard.clearHint" : "clipboard.clearImagesHint")}
                aria-label={t(tab === "text" ? "clipboard.clearHint" : "clipboard.clearImagesHint")}
                disabled={clearing || (count === 0 && failure !== "clearFailed")}
                aria-busy={clearing}
                onClick={() => { void act({ kind: "clear", what: tab }); }}>
                <EraseIcon />
              </button>
            </div>
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

export function ClipboardWidget() {
  const { effectiveDesign } = useLook();
  return effectiveDesign === "reference" ? <ReferenceClipboard /> : <AppClipboard />;
}
