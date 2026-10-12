import { Button } from "../../components/Button";
import { Check } from "../../components/Check";
import { resetPlaces, setFlag, useSettings } from "../../lib/settings";
import { useText } from "../../lib/i18n";

/**
 * Поведение окон: рубильники и спасательная кнопка.
 *
 * Первые два рубильника продублированы в трее — их дёргают чаще всего и не
 * глядя, лезть ради них в окно было бы издевательством.
 *
 * Пояснения — подсказкой при наведении (`Hints.tsx`), а не строкой под
 * рубильником: по имени тут понятно почти всё.
 *
 * Своего раздела в колонке у них больше нет: с 7 октября они стоят в
 * «Общих», под чертой и подзаголовком «Окна» (`GeneralSection`). Поля
 * отдаются без своей обёртки `rows` — их ряды уже внутри общей.
 */
export function WindowsFields() {
  const settings = useSettings();
  const t = useText();

  return (
    <>
      <label className="switch" data-hint={t("settings.windows.snappingHint")}>
        <Check
          checked={settings.snapping}
          label={t("settings.windows.snapping")}
          quiet
          onChange={(next) => setFlag("snapping", next)}
        />
        <span className="switch__text">
          <span className="switch__name">{t("settings.windows.snapping")}</span>
        </span>
      </label>

      <label className="switch" data-hint={t("settings.windows.lockHint")}>
        <Check
          checked={settings.locked}
          label={t("tray.lock")}
          quiet
          onChange={(next) => setFlag("locked", next)}
        />
        <span className="switch__text">
          <span className="switch__name">{t("tray.lock")}</span>
        </span>
      </label>

      <label className="switch" data-hint={t("settings.windows.hideHint")}>
        <Check
          checked={settings.hiddenAll}
          label={t("tray.hideAll")}
          quiet
          onChange={(next) => setFlag("hidden_all", next)}
        />
        <span className="switch__text">
          <span className="switch__name">{t("tray.hideAll")}</span>
        </span>
      </label>

      <label className="switch" data-hint={t("settings.windows.autostartHint")}>
        <Check
          checked={settings.autostart}
          label={t("settings.windows.autostart")}
          quiet
          onChange={(next) => setFlag("autostart", next)}
        />
        <span className="switch__text">
          <span className="switch__name">{t("settings.windows.autostart")}</span>
        </span>
      </label>

      <div className="field" data-hint={t("settings.windows.resetNote")}>
        <Button variant="outline" onClick={resetPlaces}>
          {t("settings.windows.reset")}
        </Button>
      </div>
    </>
  );
}
