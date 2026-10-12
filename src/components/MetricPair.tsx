import type { ReactNode } from "react";
import { NO_VALUE } from "../lib/format";
import { MetricLabel } from "./MetricLabel";
import "./MetricPair.css";

type Props = {
  label: string;
  icon?: ReactNode;
  first?: ReactNode;
  second?: ReactNode;
  /** Подписи столбцов: «ядро» и «память», «1» и «2». */
  firstHint?: string;
  secondHint?: string;
  /**
   * У показателя одна величина, а не две: второй столбец остаётся пустым.
   *
   * Именно пустым, а не прочерком: прочерк означает «датчик есть, значения
   * нет», и у частоты процессора он врал бы — второй частоты у него не
   * бывает вовсе. Строка при этом остаётся строкой пары, поэтому первое
   * значение стоит в том же столбце, что у соседних строк, а высота совпадает
   * с ними до точки.
   */
  solo?: boolean;
};

function Cell({ value, hint }: { value?: ReactNode; hint?: string }) {
  const empty = value === undefined || value === null || value === "";

  return (
    <span className="pair__cell">
      <span className={empty ? "pair__value pair__value--empty tabular" : "pair__value tabular"}>
        {empty ? NO_VALUE : value}
      </span>
      {hint !== undefined && <span className="pair__hint">{hint}</span>}
    </span>
  );
}

export function MetricPair({
  label,
  icon,
  first,
  second,
  firstHint,
  secondHint,
  solo = false,
}: Props) {
  return (
    <div className="pair">
      <span className="pair__label"><MetricLabel icon={icon}>{label}</MetricLabel></span>
      <Cell value={first} hint={firstHint} />
      {solo ? <span className="pair__cell" /> : <Cell value={second} hint={secondHint} />}
    </div>
  );
}
