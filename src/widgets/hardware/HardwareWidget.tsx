import type { ReactNode } from "react";
import { useTelemetry, type SourceState } from "../../lib/telemetry";
import { useWindowDrag } from "../../lib/drag";
import { useText } from "../../lib/i18n";
import { StatusDot } from "../../components/StatusDot";
import { GpuSection } from "./GpuSection";
import { CpuSection } from "./CpuSection";
import { DisksSection } from "./DisksSection";
import "./widget.css";

/**
 * Обвязка окна с железом: одна карточка на окно плюс объяснение, когда
 * данных нет. Без него все значения молча превратились бы в прочерки.
 */
function HardwareWidget({ children }: { children: ReactNode }) {
  const state = useTelemetry();
  useWindowDrag();

  return (
    <div className="widget">
      <SourceBanner state={state} />
      {children}
    </div>
  );
}

function SourceBanner({ state }: { state: SourceState }) {
  const t = useText();
  if (state.status === "ok") return null;

  // «Не нужны» виджет железа видит миг после своего же включения: опрос
  // просыпается раз в полсекунды. Для карточки это то же ожидание
  if (state.status === "connecting" || state.status === "idle") {
    return (
      <div className="widget__banner widget__banner--waiting" data-accent="neutral">
        <StatusDot />
        {t("hardware.connecting")}
      </div>
    );
  }

  return (
    <div className="widget__banner" data-accent="danger">
      <StatusDot glow />
      {t("hardware.silent")}
    </div>
  );
}

/*
 * Каждый показатель — своё окно: их двигают и включают порознь. Тянутся они
 * за любое пустое место карточки, этим занят useWindowDrag в оболочке.
 */

export function GpuWidget() {
  return (
    <HardwareWidget>
      <GpuSection />
    </HardwareWidget>
  );
}

export function CpuWidget() {
  return (
    <HardwareWidget>
      <CpuSection />
    </HardwareWidget>
  );
}

/*
 * Сети тут больше нет: скорости переехали в собственную карточку вместе с
 * откликом, и полосы «LibreHardwareMonitor is not responding» у них теперь нет —
 * в карточке высотой полтора десятка строк она съела бы пятую часть.
 */

export function DisksWidget() {
  return (
    <HardwareWidget>
      <DisksSection />
    </HardwareWidget>
  );
}
