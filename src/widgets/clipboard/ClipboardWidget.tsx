import { useEffect, useRef, useState } from "react";
import { useWindowDrag } from "../../lib/drag";
import { useText } from "../../lib/i18n";
import { control, controlConfirmed, useClipboard, type Entry } from "../../lib/clipboard";
import { useLook } from "../../lib/look";
import { CloseIcon, EraseIcon, FailedIcon } from "../../components/Icons";
import { GlassCard } from "../../components/GlassCard";
import { Button } from "../../components/Button";
import "./clipboard.css";
import "./clipboard-design.css";

/**
 * Сколько держим свою пометку, не давая событию её перебить.
 *
 * Тот же приём, что у виджета звука: между нажатием и подтверждением лежит
 * открытие буфера, а оно ждёт чужое приложение. Без опережения строка
 * подсвечивалась бы с заметной задержкой после щелчка.
 */
const HOLD_MS = 400;


type RowProps = {
  entry: Entry;
  current: boolean;
  onCopy: () => void;
  onDrop: () => void;
};

function Row({ entry, current, onCopy, onDrop }: RowProps) {
  const t = useText();
  return (
    <li className="clip__item" data-current={current}>
      {/*
        Две кнопки рядом, а не крестик внутри строки: кнопка в кнопке
        недопустима, да и нажатие по крестику всплывало бы в копирование.
      */}
      <button
        type="button"
        className="clip__text"
        title={`${entry.preview}\n${t.count("clipboard.chars", entry.chars)}`}
        onClick={onCopy}
      >
        {entry.preview}
      </button>
      <button type="button" className="clip__drop" title={t("common.remove")} onClick={onDrop}>
        ✕
      </button>
    </li>
  );
}

function AppClipboard() {
  useWindowDrag();
  const t = useText();
  const { entries, capacity } = useClipboard();
  const [picked, setPicked] = useState<number | undefined>(undefined);
  // Очистку жмут ради того, чтобы секрет ушёл из буфера: отказ обязан быть
  // виден. Прежде он уходил только в консоль, а список пустел — как успех
  const [clearFailed, setClearFailed] = useState(false);

  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  function copy(id: number) {
    setPicked(id);
    window.clearTimeout(timer.current);
    // Дальше пометку ведёт уже пришедшее состояние
    timer.current = window.setTimeout(() => setPicked(undefined), HOLD_MS);

    control({ kind: "copy", id });
  }

  async function clear() {
    setClearFailed(false);
    const result = await controlConfirmed({ kind: "clear" });
    setClearFailed(!result.ok);
  }

  const current = picked ?? entries.find((entry) => entry.current)?.id;

  return (
    <div className="widget">
      <GlassCard accent="neutral">
        <div className="clip">
          {entries.length === 0 ? (
            /* Не «буфер пуст»: содержимое там, возможно, и есть — просто
               скопировано до запуска, и в список оно намеренно не попало */
            <p className="clip__empty">{t("clipboard.empty")}</p>
          ) : (
            <ul className="clip__list">
              {entries.map((entry) => (
                <Row
                  key={entry.id}
                  entry={entry}
                  current={entry.id === current}
                  onCopy={() => copy(entry.id)}
                  onDrop={() => control({ kind: "remove", id: entry.id })}
                />
              ))}
            </ul>
          )}

          <div className="clip__foot">
            <span className="clip__count tabular" data-error={clearFailed || undefined} role={clearFailed ? "alert" : undefined}>
              {clearFailed ? t("clipboard.clearFailed") : t("clipboard.count", { count: entries.length, capacity })}
            </span>
            <Button
              variant="outline"
              icon={<EraseIcon />}
              label={t("clipboard.clearHint")}
              disabled={entries.length === 0 && !clearFailed}
              onClick={() => void clear()}
            />
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

/** Утверждённый A: читаемые строки; исходные Row и AppClipboard выше сохранены. */
function ReferenceRow({ entry, current, pending, failed, onCopy, onDrop }: RowProps & {
  pending: boolean;
  failed: boolean;
}) {
  const t = useText();
  // Длинный отступ в первых 200 знаках не должен делать строку пустой.
  const preview = entry.formattedPreview?.trim() ? entry.formattedPreview : entry.preview;
  const title = [preview, t.count("clipboard.chars", entry.chars),
    ...(entry.lines > 1 ? [t.count("clipboard.linesShort", entry.lines)] : [])].join("\n");
  return (
    <li className="clip-ref__item" data-current={current} data-failed={failed}>
      <button
        type="button"
        className="clip-ref__text"
        title={title}
        aria-busy={pending}
        aria-current={current ? "true" : undefined}
        onClick={onCopy}
      >
        <span className="clip-ref__preview">{preview}</span>
      </button>
      <span className="clip-ref__meta tabular">
        <span title={t.count("clipboard.chars", entry.chars)}>
          {t.count("clipboard.charsShort", entry.chars)}
        </span>
        {entry.lines > 1 && <span>{t.count("clipboard.linesShort", entry.lines)}</span>}
      </span>
      <button
        type="button"
        className="clip-ref__drop"
        title={t("common.remove")}
        aria-label={t("common.remove")}
        onClick={onDrop}
      >
        <CloseIcon />
      </button>
    </li>
  );
}

function ReferenceClipboard() {
  useWindowDrag();
  const t = useText();
  const { entries, capacity } = useClipboard();
  const [pending, setPending] = useState<number | undefined>(undefined);
  const [failed, setFailed] = useState<number | undefined>(undefined);
  const [clearFailed, setClearFailed] = useState(false);
  const [clearing, setClearing] = useState(false);
  const request = useRef(0);
  useEffect(() => () => { request.current += 1; }, []);

  async function copy(id: number) {
    const ticket = ++request.current;
    setPending(id);
    setFailed(undefined);
    setClearFailed(false);
    setClearing(false);
    const result = await controlConfirmed({ kind: "copy", id });
    // Ответ старого нажатия или разобранного дизайна уже не меняет карточку.
    if (ticket !== request.current) return;
    setPending(undefined);
    if (!result.ok) setFailed(id);
    // Текущую запись ведёт только снимок Rust после успешной записи в буфер.
  }

  function remove(id: number) {
    request.current += 1;
    setPending(undefined);
    setFailed(undefined);
    setClearing(false);
    control({ kind: "remove", id });
  }

  async function clear() {
    const ticket = ++request.current;
    setPending(undefined);
    setFailed(undefined);
    setClearFailed(false);
    setClearing(true);
    const result = await controlConfirmed({ kind: "clear" });
    if (ticket !== request.current) return;
    setClearing(false);
    setClearFailed(!result.ok);
    // Rust забывает историю даже при отказе ОС. Повтор доступен и без строк.
  }

  const failureText = clearFailed ? t("clipboard.clearFailed") : t("clipboard.copyFailed");

  return (
    <div className="widget clip-widget--reference">
      <GlassCard accent="neutral" flush>
        <div className="clip-ref">
          <div className="clip-ref__viewport">
            {entries.length === 0 ? (
              <p className="clip-ref__empty">{t("clipboard.empty")}</p>
            ) : (
              <ul
                className="clip-ref__list"
                tabIndex={0}
                aria-label={t("widget.clipboard")}
                onMouseDown={(event) => {
                  // Бегунок держит мышь, пустота и подписи по-прежнему двигают окно.
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
                    onCopy={() => { void copy(entry.id); }}
                    onDrop={() => remove(entry.id)}
                  />
                ))}
              </ul>
            )}
          </div>
          <div className="clip-ref__foot">
            {(failed !== undefined || clearFailed) && (
              <div className="clip-ref__error" role="status" title={failureText}>
                <FailedIcon />
                <span>{failureText}</span>
              </div>
            )}
            <div className="clip-ref__actions">
              <span className="clip-ref__count tabular">
                {t("clipboard.count", { count: entries.length, capacity })}
              </span>
              <button
                type="button"
                className="clip-ref__clear"
                title={t("clipboard.clearHint")}
                aria-label={t("clipboard.clearHint")}
                disabled={clearing || (entries.length === 0 && !clearFailed)}
                aria-busy={clearing}
                onClick={() => { void clear(); }}
              >
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
