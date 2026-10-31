import { useRef, type CSSProperties } from "react";
import { GlassCard } from "../../components/GlassCard";
import { DoneIcon, FailedIcon } from "../../components/Icons";
import type { Key, Text } from "../../lib/i18n";
import { openWidgetSettings } from "../../lib/settings";
import { useFittedTotpHeight } from "./fit";
import { grouped, titleOf, type Code, type Codes, type Row } from "../../lib/totp";

/** Ответ именно записи в буфер; раскрытие само по себе успехом не считается. */
export type TotpCopyFeedback = { id: string; ok: boolean };

type Props = {
  t: Text;
  accounts: Row[];
  /** Что сказать пустой карточке: «записей нет» или беда с файлом. */
  empty: { title: Key; hint: Key };
  answer: Codes | null;
  current: string | null;
  feedback: TotpCopyFeedback | null;
  hidden: boolean;
  second: number;
  layout: number;
};

function Feedback({ feedback, t, compact = false }: {
  feedback: TotpCopyFeedback;
  t: Text;
  compact?: boolean;
}) {
  const label = t(feedback.ok ? "common.copied" : "totp.copyFailed");
  return (
    <span
      className={compact ? "totp-ref__feedback-mini" : "totp-ref__feedback"}
      data-failed={!feedback.ok}
      title={label}
      role={compact ? undefined : "status"}
      aria-label={compact ? undefined : label}
      aria-hidden={compact ? true : undefined}
    >
      {feedback.ok ? <DoneIcon /> : <FailedIcon />}
      {!compact && <span className="totp-ref__feedback-text">{label}</span>}
    </span>
  );
}

/** Числа/точки не уступают место подтверждению, в отличие от исходного Classic. */
function codeText(t: Text, code: Code | undefined, row: Row, open: boolean): string {
  if (code === undefined) return "·".repeat(row.digits);
  if (code.code === null) return t("totp.unavailable");
  return open ? grouped(code.code) : "•".repeat(row.digits);
}

/** A/B — только визуал; коды, выбор и будильник принадлежат TotpWidget. */
export function TotpReference({ t, accounts, empty, answer, current, feedback, hidden, second, layout }: Props) {
  const stacked = layout === 1;
  const space = useRef<HTMLDivElement>(null);
  const sample = useRef<HTMLLIElement>(null);
  const fitted = useFittedTotpHeight(space, sample, accounts.length);

  return (
    <div className="widget totp-widget--reference">
      <GlassCard accent="neutral" flush>
        <div className="totp-ref" data-layout={stacked ? "b" : "a"}>
          {accounts.length === 0 ? (
            <button type="button" className="totp-ref__empty" data-drags onClick={() => openWidgetSettings("totp")}>
              <span>{t(empty.title)}</span>
              <span className="totp-ref__empty-hint">{t(empty.hint)}</span>
            </button>
          ) : (
            <>
              <div className="totp-ref__viewport" ref={space}>
                <ul
                  className="totp-ref__list hover-scrollbar"
                  style={fitted === undefined ? undefined : { height: fitted }}
                  aria-label={t("widget.totp")}
                  onMouseDown={(event) => {
                    // За бегунок крутят список, подписи и строки остаются ручкой окна.
                    const list = event.currentTarget;
                    const scrollbar = list.offsetWidth - list.clientWidth;
                    if (scrollbar > 0 && event.clientX >= list.getBoundingClientRect().right - scrollbar) {
                      event.stopPropagation();
                    }
                  }}
                >
                  {accounts.map((row, at) => {
                    const code = answer?.codes.find((found) => found.id === row.id);
                    const open = !hidden || current === row.id;
                    const left = code === undefined ? 0 : Math.max(0, code.validUntil - second);
                    const share = row.period > 0 ? Math.min(1, left / row.period) : 0;
                    const status = feedback?.id === row.id ? feedback : null;
                    const title = [titleOf(row), code?.code === null ? t("totp.unavailable") :
                      open ? t("totp.copy") : t("totp.revealCopy"),
                      ...(status === null ? [] : [t(status.ok ? "common.copied" : "totp.copyFailed")])].join("\n");
                    const digits = (
                      <span
                        className="totp-ref__code tabular"
                        data-open={open && code !== undefined}
                        data-unavailable={code?.code === null}
                        data-digits={row.digits}
                      >
                        {codeText(t, code, row, open)}
                      </span>
                    );

                    return (
                      <li
                        key={row.id}
                        className="totp-ref__item"
                        ref={at === 0 ? sample : undefined}
                        data-current={current === row.id}
                      >
                        {/* Не button: useCardClick и useWindowDrag делят привычный жест. */}
                        <div className="totp-ref__row" data-id={row.id} title={title}>
                          <span className="totp-ref__identity" data-label={row.issuer !== "" && row.label !== ""}>
                            <span className="totp-ref__issuer">{row.issuer === "" ? row.label : row.issuer}</span>
                            {!stacked && status !== null ? <Feedback feedback={status} t={t} /> :
                              row.issuer !== "" && row.label !== "" && <span className="totp-ref__label">{row.label}</span>}
                          </span>

                          {stacked ? (
                            <span className="totp-ref__bottom">
                              {digits}
                              {status !== null ? <Feedback feedback={status} t={t} /> : (
                                <span className="totp-ref__meter" style={{ "--left": share } as CSSProperties} aria-hidden="true">
                                  <span className="totp-ref__track"><span /></span>
                                  <span className="totp-ref__count tabular">{left}</span>
                                </span>
                              )}
                            </span>
                          ) : (
                            <>
                              <span className="totp-ref__code-block">
                                {digits}
                                {status !== null && <Feedback feedback={status} t={t} compact />}
                              </span>
                              <span className="totp-ref__ring" style={{ "--left": share } as CSSProperties} aria-hidden="true">
                                <span className="totp-ref__seconds tabular">{left}</span>
                              </span>
                            </>
                          )}
                        </div>
                      </li>
                    );
                  })}
                </ul>
              </div>
              {answer?.clockSuspect === true && (
                <p className="totp-ref__clock" title={t("totp.checkClock")}>{t("totp.checkClock")}</p>
              )}
            </>
          )}
        </div>
      </GlassCard>
    </div>
  );
}
