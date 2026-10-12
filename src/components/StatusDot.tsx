import type { Accent } from "../lib/accent";
import "./StatusDot.css";

type Props = {
  /** Не задан — берётся акцент родителя. */
  accent?: Accent;
  glow?: boolean;
};

export function StatusDot({ accent, glow = false }: Props) {
  return <span className={glow ? "dot dot--glow" : "dot"} data-accent={accent} />;
}
