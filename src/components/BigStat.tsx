import type { ReactNode } from "react";
import { NO_VALUE } from "../lib/format";
import "./BigStat.css";

type Props = {
  /** `null` или `undefined` — прочерк вместо числа. */
  value?: ReactNode;
  unit?: string;
  caption?: ReactNode;
  /**
   * Единица рядом с числом (виджет процессора и памяти) либо под ним
   * (виджет сети). Различие взято прямо с референсов.
   */
  unitPlacement?: "inline" | "below";
  /**
   * Ступень общей шкалы, а не своё число: `sm` — это `--text-xl`.
   *
   * Мельче обычного стоит в слитой карточке сети, где показателей пять и все
   * они в двух строках.
   */
  size?: "sm" | "md" | "lg";
  align?: "start" | "center";
};

export function BigStat({
  value,
  unit,
  caption,
  unitPlacement = "inline",
  size = "md",
  align = "start",
}: Props) {
  const empty = value === undefined || value === null || value === "";

  const classes = [
    "stat",
    size === "lg" ? "stat--lg" : "",
    size === "sm" ? "stat--sm" : "",
    align === "center" ? "stat--center" : "",
    unitPlacement === "below" ? "stat--stacked" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes}>
      <div className="stat__main">
        <span className={empty ? "stat__value stat__value--empty tabular" : "stat__value tabular"}>
          {empty ? NO_VALUE : value}
        </span>
        {unit !== undefined && <span className="stat__unit">{unit}</span>}
      </div>
      {caption !== undefined && <span className="stat__caption tabular">{caption}</span>}
    </div>
  );
}
