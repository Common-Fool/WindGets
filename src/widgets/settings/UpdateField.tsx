import { Button } from "../../components/Button";
import { Check } from "../../components/Check";
import { fileSize } from "../../lib/format";
import { useText } from "../../lib/i18n";
import {
  cancelUpdate,
  checkUpdate,
  checkedText,
  installUpdate,
  openReleases,
  setUpdateAuto,
  updateFailureText,
  updateJobText,
  useUpdate,
} from "../../lib/update";

/**
 * Обновления в «Общих»: версия, найденный выпуск и кнопки.
 *
 * Сами в сеть ходим раз в неделю и только при галочке; ставим только по
 * нажатию. Что произойдёт по «Обновить» — абзацем, а не подсказкой: окно
 * закроется, и человек должен знать это до нажатия.
 */
export function UpdateField() {
  const t = useText();
  const update = useUpdate();
  const job = update.job;
  const busy = job !== null;
  const offer = update.available;

  return (
    <>
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("update.version", { version: update.current })}</span>
          <span className="field__value">
            {offer !== null
              ? t("update.available", { version: offer.version })
              : update.failure === null
                ? checkedText(t, update)
                : ""}
          </span>
        </div>

        {offer !== null && !busy && <p className="note">{t("update.note")}</p>}

        {busy ? (
          <div className="choices">
            <p className="note">{updateJobText(t, job)}</p>
            {(job.stage === "checking" || job.stage === "downloading") && (
              <Button variant="outline" onClick={() => void cancelUpdate()}>
                {t("common.cancel")}
              </Button>
            )}
          </div>
        ) : (
          <div className="choices">
            {offer !== null && (
              <Button onClick={() => void installUpdate().catch(() => {})}>
                {t("update.install")} · {fileSize(offer.size)}
              </Button>
            )}
            <Button variant="outline" onClick={() => void checkUpdate()}>
              {t("update.checkNow")}
            </Button>
            <Button variant="outline" onClick={() => openReleases(update)}>
              {t("update.whatsNew")}
            </Button>
          </div>
        )}

        {!busy && update.failure !== null && (
          <p className="note note--error">{updateFailureText(t, update.failure)}</p>
        )}
      </div>

      <label className="switch" data-hint={t("update.autoHint")}>
        <Check checked={update.auto} label={t("update.auto")} quiet onChange={(next) => void setUpdateAuto(next)} />
        <span className="switch__text">
          <span className="switch__name">{t("update.auto")}</span>
        </span>
      </label>
    </>
  );
}
