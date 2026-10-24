import { useState } from "react";
import { Button } from "../../components/Button";
import { Slider } from "../../components/Slider";
import { openConfigDir, openLogDir, setSource, useSettings } from "../../lib/settings";
import { useTelemetry } from "../../lib/telemetry";
import { useText } from "../../lib/i18n";
import { exportBackup, inspectBackup, restoreBackup, type BackupError, type BackupSummary } from "../../lib/backup";

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
  const [confirmation, setConfirmation] = useState<"export" | BackupSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<BackupError | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  function clearBackupFeedback() {
    setError(null);
    setSaved(null);
    setConfirmation(null);
  }

  async function saveBackup() {
    if (busy || confirmation !== "export") return;
    setBusy(true);
    clearBackupFeedback();
    const result = await exportBackup();
    if (result.ok) setSaved(result.value);
    else setError(result.error);
    setBusy(false);
  }

  async function chooseBackup() {
    if (busy) return;
    setBusy(true);
    clearBackupFeedback();
    const result = await inspectBackup();
    if (result.ok) setConfirmation(result.value);
    else setError(result.error);
    setBusy(false);
  }

  async function applyBackup() {
    if (busy || confirmation === null || confirmation === "export") return;
    setBusy(true);
    setError(null);
    const result = await restoreBackup();
    if (!result.ok) {
      setError(result.error);
      setBusy(false);
    }
    // После успеха ждём перезапуска: повторно восстанавливать уже нельзя.
  }

  const summary = confirmation !== "export" ? confirmation : null;
  const created = summary ? new Date(summary.created) : null;
  const createdText = created && Number.isFinite(created.getTime())
    ? new Intl.DateTimeFormat(t.locale, { dateStyle: "short", timeStyle: "short" }).format(created)
    : summary?.created ?? "";

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

      <hr className="divider" />
      <h3 className="settings__subheading caps">{t("backup.heading")}</h3>
      <div className="field" aria-busy={busy}>
        <p className="note">{t("backup.about")}</p>
        <div className="choices">
          <Button variant="outline" disabled={busy} onClick={() => {
            clearBackupFeedback();
            setConfirmation("export");
          }}>
            {t("backup.export")}
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => void chooseBackup()}>
            {t("backup.restore")}
          </Button>
        </div>

        {confirmation === "export" && (
          <div className="field">
            <p className="note" role="alert">{t("backup.warning")}</p>
            <div className="choices">
              <Button disabled={busy} onClick={() => void saveBackup()}>{t("backup.exportConfirm")}</Button>
              <Button variant="outline" disabled={busy} onClick={clearBackupFeedback}>{t("common.cancel")}</Button>
            </div>
          </div>
        )}

        {summary !== null && (
          <div className="field">
            <p className="note">{t("backup.summary", { file: summary.file, created: createdText, app: summary.app })}</p>
            <p className="note">{t("backup.summaryContents", {
              mail: summary.mail, totp: summary.totp, notes: summary.notes,
              todo: summary.todo, widgets: summary.widgets,
            })}</p>
            {summary.spotify && <p className="note">{t("backup.summarySpotify")}</p>}
            <p className="note" role="alert">{t("backup.restoreConfirm")}</p>
            <div className="choices">
              <span className="totp-remove">
                <Button disabled={busy} onClick={() => void applyBackup()}>{t("backup.restoreButton")}</Button>
              </span>
              <Button variant="outline" disabled={busy} onClick={clearBackupFeedback}>{t("common.cancel")}</Button>
            </div>
          </div>
        )}

        {saved !== null && (
          <p className="note" role="status" data-no-drag style={{ userSelect: "text", overflowWrap: "anywhere" }}>
            {t("backup.saved", { path: saved })}
          </p>
        )}
        {error !== null && (
          <p className="note note--error" role="alert">
            {t(`backup.error.${error.code}`, { file: error.file ?? "" })}
          </p>
        )}
      </div>
    </div>
  );
}
