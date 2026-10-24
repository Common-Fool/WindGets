import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check } from "../../components/Check";
import { useBattery } from "../../lib/battery";
import { useText } from "../../lib/i18n";
import { useSettings } from "../../lib/settings";
import { ComponentsField } from "./ToolsFields";

/** Выбор периферии и докачиваемая программа для гарнитур. */
export function BatterySection() {
  const t = useText();
  const { devices } = useBattery();
  const { hiddenBatteries } = useSettings();
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const requested = useRef(pending);
  const [failed, setFailed] = useState(false);

  // До подтверждающего снимка учитываем уже нажатые галочки. Ref закрывает
  // и два быстрых нажатия до следующего рендера.
  useEffect(() => {
    const next = { ...requested.current };
    for (const [id, hidden] of Object.entries(next)) {
      if (hiddenBatteries.includes(id) === hidden) delete next[id];
    }
    requested.current = next;
    setPending(next);
  }, [hiddenBatteries]);

  const isHidden = (id: string) => pending[id] ?? hiddenBatteries.includes(id);
  const present = devices.map((device) => device.id);
  const shown = devices.filter((device) => !isHidden(device.id));
  // Отключённые скрытые устройства остаются в списке для возврата.
  const ids = [...new Set([...present, ...hiddenBatteries, ...Object.keys(pending)])];

  const toggle = (id: string, show: boolean) => {
    if (id in requested.current) return;
    const hiddenNow = (key: string) => requested.current[key] ?? hiddenBatteries.includes(key);
    if (!show && !hiddenNow(id) && present.filter((key) => !hiddenNow(key)).length <= 1) return;

    const next = { ...requested.current, [id]: !show };
    requested.current = next;
    setPending(next);
    setFailed(false);
    void invoke("set_battery_hidden", { id, hidden: !show }).catch(() => {
      const remaining = { ...requested.current };
      delete remaining[id];
      requested.current = remaining;
      setPending(remaining);
      setFailed(true);
    });
  };

  return (
    <div className="rows">
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("battery.settings.devices")}</span>
          <span className="field__value tabular">{shown.length} / {devices.length}</span>
        </div>
        {devices.length === 0 && <p className="note">{t("battery.settings.none")}</p>}
        {ids.length > 0 && (
          <div className="rows rows--tight">
            {ids.map((id) => {
              const device = devices.find((entry) => entry.id === id);
              const name = device?.name || id;
              const last = device !== undefined && !isHidden(id) && shown.length === 1;
              return (
                <label key={id} className="switch" data-hint={last ? t("battery.settings.lastShown") : undefined}>
                  <Check checked={!isHidden(id)} label={name} quiet
                    disabled={last || id in pending} onChange={(show) => toggle(id, show)} />
                  <span className="switch__text">
                    <span className="switch__name" title={name}>{name}</span>
                    <span className="switch__hint">
                      {device === undefined ? t("disks.settings.absent") : (
                        <>{t(`battery.kind.${device.kind}`)} · {t(`battery.source.${device.source}`)}</>
                      )}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
        )}
        {failed && <p className="note note--error" role="alert">{t("common.failed")}</p>}
      </div>
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("battery.settings.headset")}</span>
        </div>
        <ComponentsField module="battery" extra={t("battery.settings.headsetNote")} />
      </div>
    </div>
  );
}
