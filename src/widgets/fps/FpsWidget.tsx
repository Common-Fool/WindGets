import { useWidgetView } from "../../lib/view";
import { useFps, type FpsView } from "../../lib/fps";
import { missingFor, useTools } from "../../lib/tools";
import { useText } from "../../lib/i18n";
import { NO_VALUE } from "../../lib/format";
import { GlassCard } from "../../components/GlassCard";
import { BigStat } from "../../components/BigStat";
import { Columns } from "../../components/Columns";
import { ToolsGate } from "../../components/ToolsGate";
import "./fps.css";

/** Целое число или прочерк: десятые доли FPS только дрожат. */
function whole(value: number | null): string {
  return value === null ? NO_VALUE : Math.round(value).toString();
}

/** Миллисекунды кадра: на высоких FPS вся разница в десятых. */
function ms(value: number | null): string {
  if (value === null) return NO_VALUE;
  return value < 10 ? value.toFixed(1) : value.toFixed(0);
}

/**
 * Верх шкалы истории: ступенями по 30 кадров, не ниже 60. Шкала от самого
 * большого значения прыгала бы на каждой загрузке уровня.
 */
function ceiling(history: readonly number[]): number {
  const top = history.reduce((max, value) => Math.max(max, value), 0);
  return Math.max(60, Math.ceil(top / 30) * 30);
}

function Stat({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <div className="fps__stat">
      <dt>{label}</dt>
      <dd className="tabular">
        {value}
        {unit !== undefined && value !== NO_VALUE && <span className="fps__unit">{unit}</span>}
      </dd>
    </div>
  );
}

function Head({ view }: { view: FpsView }) {
  const t = useText();
  const title =
    view.status === "live" ? view.app ?? NO_VALUE
      : view.status === "failed" ? t("fps.failed")
        : NO_VALUE;
  // Способ вывода на карточке не нужен: он отвечает «полноэкранный ли вывод»,
  // а не «как идёт игра». Остаётся подсказкой у имени
  const hint = view.status === "failed" ? view.problem : view.mode;

  return (
    <div className="fps__head">
      <span className="fps__app" title={hint ?? undefined}>{title}</span>
    </div>
  );
}

/** Квадрат с одним числом: FPS и подпись под ним. */
function Square({ view }: { view: FpsView }) {
  return (
    <div className="fps fps--square" title={view.app ?? undefined}>
      <span className="fps__square-value tabular">{whole(view.fps)}</span>
      <span className="fps__square-unit">FPS</span>
    </div>
  );
}

/**
 * Счётчик кадров игры на переднем плане.
 *
 * Крупно — FPS за последнюю секунду, сбоку — 1 % и 0,1 % low, время кадра и
 * занятость видеокарты (около сотни — упор в неё, заметно ниже — в процессор
 * или ограничитель). Под ними история по секундам; ниже порога высоты она
 * уходит. Второй вид — квадрат с одним числом. Игру выбирает Rust
 * (`src-tauri/src/fps`), обновление раз в секунду.
 */
export function FpsWidget() {
  const tools = useTools();
  const view = useFps();
  const look = useWidgetView();

  if (missingFor(tools, "fps").length > 0) {
    return (
      <div className="widget">
        <GlassCard accent="neutral">
          <ToolsGate module="fps" />
        </GlassCard>
      </div>
    );
  }

  return (
    <div className="widget">
      <GlassCard accent="gpu" flush>
        {look === 2 ? <Square view={view} /> : (
        <div className="fps">
          <Head view={view} />
          <div className="fps__main">
            <BigStat size="lg" value={view.fps === null ? undefined : whole(view.fps)} unit="FPS" />
            <dl className="fps__stats">
              <Stat label="1% low" value={whole(view.low1)} />
              <Stat label="0.1% low" value={whole(view.low01)} />
              <Stat label="Frame" value={ms(view.frameMs)} unit="ms" />
              <Stat label="GPU busy" value={whole(view.gpuBusy)} unit="%" />
            </dl>
          </div>
          {look === 0 && (
            <div className="fps__history">
              <Columns values={view.history} max={ceiling(view.history)} />
            </div>
          )}
        </div>
        )}
      </GlassCard>
    </div>
  );
}
