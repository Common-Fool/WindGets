import type { ReactNode } from "react";
import type { Accent } from "../lib/accent";
import "./GlassCard.css";

type Props = {
  /**
   * Заголовок в шапке. Без него шапки нет вовсе.
   *
   * Виджеты его не передают: окно и есть одна карточка, называть её незачем.
   * Осталось для витрины и всплывающих окон, где карточек в окне несколько.
   */
  title?: string;
  /** Значение справа в шапке — как «6.0 %» у виджета видеокарты. */
  value?: ReactNode;
  /** Заголовок по центру и без значения — как «STORAGE». */
  centered?: boolean;
  accent?: Accent;
  /** Убрать внутренние отступы тела — для содержимого во всю ширину. */
  flush?: boolean;
  children: ReactNode;
};

export function GlassCard({
  title,
  value,
  centered = false,
  accent = "neutral",
  flush = false,
  children,
}: Props) {
  return (
    <section className="card" data-accent={accent}>
      {title !== undefined && (
        <header className={centered ? "card__head card__head--centered" : "card__head"}>
          <span className="card__title caps">{title}</span>
          {value !== undefined && <span className="card__value tabular">{value}</span>}
        </header>
      )}
      <div
        className={[
          "card__body",
          flush ? "card__body--flush" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {children}
      </div>
    </section>
  );
}
