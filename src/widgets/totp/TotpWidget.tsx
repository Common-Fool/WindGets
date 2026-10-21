import { useEffect, useRef, useState } from "react";
import { GlassCard } from "../../components/GlassCard";
import { copySecret } from "../../lib/copy";
import { useWindowDrag } from "../../lib/drag";
import { useText, type Text } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useCardClick } from "../../lib/gestures";
import { openWidgetSettings } from "../../lib/settings";
import { useHeight } from "../../lib/size";
import { useWidgetLayout, useWidgetView } from "../../lib/view";
import { codesFor, emptyText, grouped, useAccounts, type Code, type Codes, type Row } from "../../lib/totp";
import "./totp.css";
import { TotpReference, type TotpCopyFeedback } from "./TotpReference";
import "./totp-reference.css";

/*
 * Коды 2FA.
 *
 * ВАЖНО: **строки — не кнопки, и это не оплошность.** Кнопка удерживает мышь
 * (`KEEPS_THE_MOUSE`), а значит за неё не потащишь окно и по ней не откроешь
 * меню виджета. Строки занимают всю карточку целиком, так что кнопками они
 * отняли бы у виджета и перетаскивание, и правую кнопку разом — что живая
 * проверка и поймала. Нажатие ловит `useCardClick`: он сам отличает щелчок от
 * начатого перетаскивания по сдвигу мыши.
 *
 * Вид меняют только меню виджета и настройки, то есть осознанно: карточка
 * висит на обоях поверх всех окон, и открытые коды не должны раскрываться
 * случайным щелчком. Щелчком по пустому месту вид не листается ни у одного
 * виджета с этапа 53.
 *
 * Опроса в Rust у этого виджета нет: коды считаются по запросу, а будильник на
 * границу периода ставит эта карточка — см. `src-tauri/src/totp/mod.rs`.
 */

/** Запас, с которым просим коды после границы периода. */
const AFTER_BOUNDARY = 250;

/** Наибольшая задержка, которую `setTimeout` отмеряет честно. */
const LONGEST_TIMER = 2 ** 31 - 1;

/** Сколько держится подпись «Copied». */
const COPIED_FOR = 2000;

export function TotpWidget() {
  useWindowDrag();
  const t = useText();
  const reference = useLook().effectiveDesign === "reference";
  const layout = useWidgetLayout();

  const { accounts, trouble } = useAccounts();
  const empty = emptyText(trouble);
  // Первый вид — скрытый: открытые коды не должны быть состоянием по умолчанию
  const hidden = useWidgetView() === 0;

  const [answer, setAnswer] = useState<Codes | null>(null);
  const [current, setCurrent] = useState<string | null>(null);
  const [copied, setCopied] = useState<{ id: string } | null>(null);
  const [copyFeedback, setCopyFeedback] = useState<TotpCopyFeedback | null>(null);
  const copyRequest = useRef(0);
  const listPosition = useRef(0);

  /*
   * Считаем остаток по часам Rust, а не по своим: ответ приносит своё `now`,
   * и разница между ним и нашими часами фиксируется один раз. Иначе кольцо
   * доходило бы до конца не тогда, когда код на самом деле сменился.
   */
  const offset = useRef(0);
  const [second, setSecond] = useState(() => Math.floor(Date.now() / 1000));

  /*
   * Строки показываем целыми: обрезанная пополам читается как поломка. Место
   * меряем у обёртки — она `flex: 1`, и её высота от числа строк не зависит,
   * поэтому замер не гоняет сам себя по кругу. Тот же приём, что у процессов.
   */
  const space = useRef<HTMLDivElement>(null);
  const sample = useRef<HTMLLIElement>(null);
  const room = useHeight(space);
  const pitch = useHeight(sample);
  const fitted = pitch > 0 ? Math.max(Math.floor(room / pitch), 1) * pitch : undefined;

  const ids = accounts.map((row) => row.id).join(",");

  // Компонент модели не меняется вместе с дизайном: выбор и коды сохраняются.
  // Прокрутка также переживает переход между исходным и новым списком.
  useEffect(() => {
    const list = document.querySelector<HTMLElement>(reference ? ".totp-ref__list" : ".totp__list");
    if (list === null) return;
    list.scrollTop = listPosition.current;
    const remember = () => { listPosition.current = list.scrollTop; };
    list.addEventListener("scroll", remember);
    return () => list.removeEventListener("scroll", remember);
  }, [reference, accounts.length]);

  useEffect(() => () => { copyRequest.current += 1; }, []);

  useEffect(() => {
    if (accounts.length === 0) {
      setAnswer(null);
      return;
    }

    let alive = true;
    let timer: number | undefined;
    // Ближайшая граница периода, секунды эпохи по часам Rust. Сверяет её
    // ежесекундный такт: будильник отмеряет промежуток, а не мгновение, и
    // переведённые вперёд часы иначе оставили бы на карточке код прежнего
    // времени почти до конца периода
    let due = Number.POSITIVE_INFINITY;

    const ask = () => {
      // Прежний будильник снимаем сразу: окно, спрятанное и показанное до
      // границы периода, иначе держало бы две петли запросов, и с каждым
      // таким разом их становилось бы вдвое больше. Нашло это ревью второй
      // модели
      window.clearTimeout(timer);
      timer = undefined;
      due = Number.POSITIVE_INFINITY;

      // Спрятанное окно не просит ничего. На пустой ответ не перезапрашиваем
      // вовсе: разбудит нас `visibilitychange`, а не плотная петля IPC
      if (document.visibilityState !== "visible") return;

      void codesFor([])
        .then((next) => {
          if (!alive) return;

          offset.current = next.now - Math.floor(Date.now() / 1000);
          setSecond(next.now);
          setAnswer(next);
          // Раскрытая строка гаснет с концом своего периода: граница и так
          // есть, и второго таймера ей не нужно
          setCurrent(null);

          const soonest = next.codes.reduce(
            (min, code) => Math.min(min, code.validUntil),
            Number.POSITIVE_INFINITY,
          );
          if (!Number.isFinite(soonest)) return;

          // Запас не украшение: запрос, ушедший за миллисекунды до границы,
          // вернул бы старый код с истёкшим `validUntil` — и мы ушли бы в петлю.
          // Отмеряем от своих часов с точностью до миллисекунды, а не от
          // `next.now`: тот усечён до секунды, и запас выходил бы до 1,25 с —
          // столько карточка показывала бы просроченный код. Сверху — предел
          // таймера: больше 2^31 мс он срабатывает сразу, и период длиной в
          // месяц превратил бы будильник в петлю запросов
          due = soonest;
          const left = soonest * 1000 - (Date.now() + offset.current * 1000);
          const wait = Math.min(Math.max(left + AFTER_BOUNDARY, AFTER_BOUNDARY), LONGEST_TIMER);
          // Будильник всегда один: два ответа внахлёст (проснулись, пока
          // прошлый запрос ещё шёл) иначе завели бы два
          window.clearTimeout(timer);
          timer = window.setTimeout(ask, wait);
        })
        .catch(() => {
          // Вне Tauri кодов нет; повторит показ окна
        });
    };

    ask();

    const wake = () => {
      if (document.visibilityState === "visible") ask();
    };
    document.addEventListener("visibilitychange", wake);

    const beat = window.setInterval(() => {
      const second = Math.floor(Date.now() / 1000) + offset.current;
      setSecond(second);
      if (second >= due) ask();
    }, 1000);

    return () => {
      alive = false;
      window.clearTimeout(timer);
      window.clearInterval(beat);
      document.removeEventListener("visibilitychange", wake);
    };
  }, [ids, accounts.length]);

  useEffect(() => {
    if (copied === null) return;
    const timer = window.setTimeout(() => setCopied(null), COPIED_FOR);
    return () => window.clearTimeout(timer);
  }, [copied]);

  // GPT получает отдельный ответ с тем же сроком, что и Copied у Claude.
  useEffect(() => {
    if (copyFeedback === null) return;
    const timer = window.setTimeout(() => setCopyFeedback(null), COPIED_FOR);
    return () => window.clearTimeout(timer);
  }, [copyFeedback]);

  // Нажатие по строке: раскрыть её и положить код в буфер
  useCardClick((target) => {
    const line = target?.closest(".totp__row, .totp-ref__row");
    if (!(line instanceof HTMLElement)) return;

    const id = line.dataset.id;
    const code = answer?.codes.find((found) => found.id === id);
    if (id === undefined || code?.code == null) return;

    setCurrent(id);
    const ticket = ++copyRequest.current;
    setCopyFeedback(null);
    void copySecret(code.code).then((done) => {
      // Поздний ответ не отменяет новое действие ни в одном дизайне.
      if (ticket !== copyRequest.current) return;
      // Новый объект перезапускает срок Copied и при повторе той же записи.
      if (done) setCopied({ id });
      setCopyFeedback({ id, ok: done });
    }).catch(() => {
      if (ticket === copyRequest.current) setCopyFeedback({ id, ok: false });
    });
  });

  if (reference) {
    return <TotpReference t={t} accounts={accounts} empty={empty} answer={answer} current={current}
      feedback={copyFeedback} hidden={hidden} second={second} layout={layout} />;
  }

  if (accounts.length === 0) {
    return (
      <div className="widget">
        <GlassCard accent="neutral">
          {/* Кнопкой намеренно: у пустой карточки одно действие, и место для
              перетаскивания вокруг неё остаётся */}
          <button type="button" className="totp__empty" data-drags onClick={() => openWidgetSettings("totp")}>
            <span className="totp__empty-title">{t(empty.title)}</span>
            <span className="totp__empty-hint">{t(empty.hint)}</span>
          </button>
        </GlassCard>
      </div>
    );
  }

  return (
    <div className="widget">
      <GlassCard accent="neutral" flush>
        <div className="totp" ref={space}>
          <ul className="totp__list hover-scrollbar" style={fitted === undefined ? undefined : { maxHeight: fitted }}>
            {accounts.map((row, at) => {
              const code = answer?.codes.find((found) => found.id === row.id);
              const open = !hidden || current === row.id;
              const left = code === undefined ? 0 : Math.max(0, code.validUntil - second);
              const share = row.period > 0 ? left / row.period : 0;

              return (
                <li
                  key={row.id}
                  className="totp__item"
                  ref={at === 0 ? sample : undefined}
                  data-current={current === row.id}
                >
                  <div
                    className="totp__row"
                    data-id={row.id}
                    title={open ? t("totp.copy") : t("totp.revealCopy")}
                  >
                    <span className="totp__name">
                      <span className="totp__issuer">
                        {row.issuer === "" ? row.label : row.issuer}
                      </span>
                      {row.issuer !== "" && row.label !== "" && (
                        <span className="totp__label">{row.label}</span>
                      )}
                    </span>

                    <span className="totp__code tabular" data-open={open}>
                      {cell(t, code, row, open, copied?.id === row.id)}
                    </span>

                    {/* Кольцо остатка: доля дорисовывается конической заливкой,
                        а не кадрами — перерисовка раз в секунду, и та не в JS */}
                    <span
                      className="totp__ring"
                      style={{ "--left": share } as React.CSSProperties}
                      aria-hidden="true"
                    >
                      <span className="totp__seconds tabular">{left}</span>
                    </span>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>

        {answer?.clockSuspect === true && <p className="totp__clock">{t("totp.checkClock")}</p>}
      </GlassCard>
    </div>
  );
}

/** Что стоит в столбце кода: подтверждение, цифры, точки или отказ. */
function cell(t: Text, code: Code | undefined, row: Row, open: boolean, copied: boolean): string {
  if (copied) return t("common.copied");
  if (code === undefined) return "·".repeat(row.digits);
  // Нерасшифрованный секрет — чаще всего перенесённый профиль Windows.
  // Прятать это за точками нельзя: человек ждал бы кода, которого не будет
  if (code.code === null) return t("totp.unavailable");
  return open ? grouped(code.code) : "•".repeat(row.digits);
}
