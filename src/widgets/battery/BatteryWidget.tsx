import { useRef } from "react";
import { GlassCard } from "../../components/GlassCard";
import { Bar } from "../../components/Bar";
import { useWindowDrag } from "../../lib/drag";
import { batteryStep, useBattery } from "../../lib/battery";
import { NO_VALUE } from "../../lib/format";
import { useContentHeight } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useSettings } from "../../lib/settings";
import { ChargingIcon, DeviceIcon } from "./icons";
import "./battery.css";
import "./battery-design.css";

/** Заряд подключённой периферии; порядок устройств задаёт снимок. */
export function BatteryWidget() {
  const { ready, devices } = useBattery();
  const { hiddenBatteries } = useSettings();
  const t = useText();
  const gpt = useLook().effectiveDesign === "reference";
  const list = useRef<HTMLDivElement>(null);
  const shown = devices.filter((device) => !hiddenBatteries.includes(device.id));
  useWindowDrag();

  // Пустое состояние тоже меряется: окно всегда обнимает одну строку.
  useContentHeight(list, true, `${gpt}:${ready ? shown.length : 0}`, true);

  return (
    <GlassCard>
      <div ref={list} className={gpt ? "battery battery--reference" : "battery"}>
        {!ready || devices.length === 0 ? (
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
  );
}
