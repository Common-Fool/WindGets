import { useRef, useState } from "react";
import { GlassCard } from "../../components/GlassCard";
import { Arc } from "../../components/Arc";
import { Bar } from "../../components/Bar";
import { Marquee } from "../../components/Marquee";
import { batteryStep, useBattery } from "../../lib/battery";
import { NO_VALUE } from "../../lib/format";
import { useContentHeight } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useOwnRow, useSettings } from "../../lib/settings";
import { ChargingIcon, DeviceIcon } from "./icons";
import "./battery.css";
import "./battery-design.css";

/** Виды по порядку `views` в реестре. */
type View = "list" | "icons" | "labeled";
const VIEWS: View[] = ["list", "icons", "labeled"];

/** Заряд подключённой периферии; порядок устройств задаёт снимок. */
export function BatteryWidget() {
  const { ready, devices } = useBattery();
  const { hiddenBatteries } = useSettings();
  const t = useText();
  const modern = useLook().effectiveDesign === "reference";
  const list = useRef<HTMLDivElement>(null);
  const shown = devices.filter((device) => !hiddenBatteries.includes(device.id));
  const view = VIEWS[useOwnRow()?.view ?? 0] ?? "list";
  // Над каким значком мышь: его имя и бежит
  const [hovered, setHovered] = useState<string | null>(null);

  // Пустое состояние тоже меряется: окно всегда обнимает одну строку.
  useContentHeight(list, true, `${modern}:${view}:${ready ? shown.length : 0}`, true);

  // ВАЖНО: **без обёртки `.widget` карточка высотой с содержимое, а не с окно:**
  // замер сравнивал бы её саму с собой и окна не ужимал, а в зазоре под ней
  // светилась бы системная подложка
  return (
    <div className="widget">
      <GlassCard>
        <div ref={list} className={modern ? "battery battery--reference" : "battery"} data-view={view}>
          {view !== "list" && ready && shown.length > 0 ? shown.map((device) => (
            // Столбик значков: заряд — кольцо вокруг значка, процент — в `title`
            <div key={device.id} className="battery__tile" data-step={batteryStep(device.level)}
              title={`${device.name} · ${Math.round(device.level)} %`}
              onMouseEnter={() => setHovered(device.id)}
              onMouseLeave={() => setHovered((id) => id === device.id ? null : id)}>
              <Arc value={device.level / 100} thickness={10}>
                <span className="battery__icon">
                  {device.charging === true ? <ChargingIcon /> : <DeviceIcon kind={device.kind} />}
                </span>
              </Arc>
              {view === "labeled" && (
                <Marquee text={device.name} className="battery__label"
                  title={`${device.name} · ${Math.round(device.level)} %`}
                  active={hovered === device.id} wholeWords />
              )}
            </div>
          )) : !ready || devices.length === 0 ? (
            <div className="battery__empty">{NO_VALUE}</div>
          ) : shown.length === 0 ? (
            <div className="battery__empty">{t("battery.allHidden")}</div>
          ) : shown.map((device) => (
            <div key={device.id} className="battery__device" data-step={batteryStep(device.level)}>
              <span className="battery__icon" title={t(`battery.kind.${device.kind}`)}>
                <DeviceIcon kind={device.kind} />
              </span>
              <span className="battery__name" title={device.name}>{device.name}</span>
              <span className="battery__reading tabular">
                {device.charging === true && (
                  <span className="battery__charging" title={t("battery.charging")} aria-label={t("battery.charging")}>
                    <ChargingIcon />
                  </span>
                )}
                <span>{Math.round(device.level)}<small> %</small></span>
              </span>
              <div className="battery__level" aria-hidden="true">
                <Bar fill={device.level / 100} thickness="thin" />
              </div>
            </div>
          ))}
        </div>
      </GlassCard>
    </div>
  );
}
