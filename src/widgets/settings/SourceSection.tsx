import { Button } from "../../components/Button";
import { Slider } from "../../components/Slider";
import { openConfigDir, openLogDir, setSource, useSettings } from "../../lib/settings";
import { useTelemetry } from "../../lib/telemetry";
import { useText } from "../../lib/i18n";

/**
 * Источник показаний: состояние сайдкара и шаг опроса.
 *
 * Адреса тут больше нет — железо читает свой сайдкар, а не внешний
 * LibreHardwareMonitor, и ходить некуда.
 *
 * Шаг стоит помнить обратной стороной: история столбиками считается в
 * отсчётах, и на шаге в две секунды пять минут превращаются в десять.
 */
export function SourceSection() {
  const settings = useSettings();
  const telemetry = useTelemetry();
  const t = useText();

  const state = (() => {
    switch (telemetry.status) {
      case "ok":
        return t("settings.source.ok");
      case "connecting":
        return t("settings.source.connecting");
      case "idle":
        return t("settings.source.idle");
      default:
        return telemetry.reason;
    }
  })();

  return (
    <div className="rows">
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("settings.source.state")}</span>
          <span className="field__value">{state}</span>
        </div>
        <p className="note">{t("settings.source.note")}</p>
      </div>

      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("settings.source.step")}</span>
          <span className="field__value tabular">
            {t.count("settings.source.seconds", settings.pollSeconds)}
          </span>
        </div>
        <Slider
          value={settings.pollSeconds}
          min={1}
          max={5}
          ariaLabel={t("settings.source.step")}
          onChange={(next) => setSource(next)}
        />
        <p className="note">{t("settings.source.stepNote")}</p>
      </div>

      <div className="field">
        <Button variant="outline" onClick={openConfigDir}>
          {t("settings.source.openFolder")}
        </Button>
        <p className="note">{t("settings.source.folderNote")}</p>
        {/* Файл настроек не прочитался при запуске: правки живут до перезапуска */}
        {settings.unsaved && <p className="note note--error" role="alert">{t("settings.source.unsaved")}</p>}
      </div>

      <div className="field">
        <Button variant="outline" onClick={openLogDir}>
          {t("settings.source.openLog")}
        </Button>
        <p className="note">{t("settings.source.logNote")}</p>
      </div>
    </div>
  );
}
