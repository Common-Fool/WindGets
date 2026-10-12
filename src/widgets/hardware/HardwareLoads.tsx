import { useText } from "../../lib/i18n";
import { Arc } from "../../components/Arc";
import { Bar } from "../../components/Bar";
import { Columns } from "../../components/Columns";
import { gigabytes, NO_VALUE } from "../../lib/format";
import type { Accent } from "../../lib/accent";
import "./hardware-design.css";

type Load = { label: string; value: number | null | undefined; fill: number | null; history: readonly number[]; accent: Accent };
type Memory = { label: string; used: number | null | undefined; total: number | null | undefined; fill: number | null; history: readonly number[] };

/** Верхняя компоновка GPT Design у пары CPU/GPU. Данные остаются у секций. */
export function HardwareLoads({ view, load, memory }: { view: number; load: Load; memory: Memory }) {
  const t = useText();
  const used = gigabytes(memory.used) ?? NO_VALUE;
  const total = gigabytes(memory.total) ?? NO_VALUE;
  const percent = load.value == null ? NO_VALUE : load.value.toFixed(0);
  const rows = [
    { label: load.label, accent: load.accent, fill: load.fill, history: load.history, value: percent, memory: false },
    { label: memory.label, accent: "ram" as const, fill: memory.fill, history: memory.history, value: used, memory: true },
  ];
  return (
    <div className="hardware-loads" data-view={view === 1 ? "bars" : "rings"}>
      {rows.map((row) => (
        <div className="hardware-load" data-accent={row.accent} key={row.label}>
          {view !== 1 && (
            <div className="hardware-load__dial">
              <Arc value={row.fill} thickness={7.5}>
                <span className={row.memory ? "hardware-load__ring-value hardware-load__ring-value--memory tabular" : "hardware-load__ring-value tabular"}>
                  {row.value}{!row.memory && <span className="hardware-load__percent">%</span>}
                </span>
                {row.memory && <span className="hardware-load__ring-total tabular">/ {total} GB</span>}
              </Arc>
            </div>
          )}
          <div className="hardware-load__content">
            <div className="hardware-load__head">
              <span className="hardware-load__label">{row.label}</span>
              {view === 1 ? (
                <span className={row.memory ? "hardware-load__current hardware-load__current--memory tabular" : "hardware-load__current tabular"}>
                  {row.value}<span className="hardware-load__unit">{row.memory ? ` / ${total} GB` : "%"}</span>
                </span>
              ) : <span className="hardware-load__history-label">{t("hardware.history")}</span>}
            </div>
            {view === 1 && row.memory && <Bar fill={row.fill} />}
            <Columns values={row.history} />
          </div>
        </div>
      ))}
    </div>
  );
}
