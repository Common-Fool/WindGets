import type { ReactNode } from "react";
import "./MetricLabel.css";

/** Значок добавляется внутри существующей подписи, без новой строки виджета. */
export function MetricLabel({
  icon,
  badge = false,
  compact,
  children,
}: {
  icon?: ReactNode;
  badge?: boolean;
  /** Короткая подпись для тесных строк; значок при этом остаётся виден. */
  compact?: string;
  children: ReactNode;
}) {
  if (!icon) return <>{children}</>;
  return (
    <span className={badge ? "metric-label metric-label--badge" : "metric-label"}>
      <span className="metric-label__icon" aria-hidden="true">{icon}</span>
      <span className="metric-label__text">
        {compact ? (
          <>
            <span className="metric-label__full">{children}</span>
            <abbr className="metric-label__compact" title={typeof children === "string" ? children : undefined}>{compact}</abbr>
          </>
        ) : children}
      </span>
    </span>
  );
}
