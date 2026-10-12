import { Button } from "../../components/Button";
import { fileSize } from "../../lib/format";
import { useText } from "../../lib/i18n";
import {
  cancelPawnio,
  failureText,
  installPawnio,
  jobText,
  openPawnioPage,
  removePawnio,
  statusText,
  usePawnio,
} from "../../lib/pawnio";

/**
 * Драйвер PawnIO на странице процессора: что это, стоит ли, и кнопки.
 *
 * Пояснение здесь абзацем, а не подсказкой: это драйвер ядра на всю систему,
 * и что ставим и зачем, человек обязан прочитать до нажатия, а не наткнуться
 * мышью. Удалить можно только поставленный Windgets — стоящий от другой
 * программы ставил человек, и на нём держится она.
 */
export function DriverField() {
  const t = useText();
  const driver = usePawnio();
  const job = driver.job;
  const busy = job !== null;
  const offer = driver.status === "missing" || driver.status === "outdated";
  const removable = driver.ours && driver.status !== "missing";

  return (
    <div className="field">
      <div className="field__head">
        <span className="field__name">{t("pawnio.title")}</span>
        <span className="field__value">{statusText(t, driver)}</span>
      </div>
      <p className="note">{t("pawnio.note")}</p>

      {busy ? (
        <div className="choices">
          <p className="note">{jobText(t, job)}</p>
          {(job.stage === "checking" || job.stage === "downloading") && (
            <Button variant="outline" onClick={() => void cancelPawnio()}>
              {t("common.cancel")}
            </Button>
          )}
        </div>
      ) : (
        <div className="choices">
          {offer && (
            <Button onClick={() => void installPawnio().catch(() => {})}>
              {t(driver.status === "outdated" ? "pawnio.update" : "pawnio.install")} · {fileSize(driver.downloadBytes)}
            </Button>
          )}
          {removable && (
            <Button variant="outline" onClick={() => void removePawnio().catch(() => {})}>
              {t("pawnio.remove")}
            </Button>
          )}
          <Button variant="outline" onClick={() => openPawnioPage(driver)}>
            {t("pawnio.more")}
          </Button>
        </div>
      )}

      {driver.status === "restart" && !busy && <p className="note">{t("pawnio.gate.restartWhy")}</p>}
      {!driver.ours && driver.status !== "missing" && driver.status !== "unknown" && (
        <p className="note">{t("pawnio.notOurs")}</p>
      )}
      {driver.ours && <p className="note">{t("pawnio.stays")}</p>}
      {!busy && driver.failure !== null && <p className="note note--error">{failureText(t, driver.failure)}</p>}
    </div>
  );
}
