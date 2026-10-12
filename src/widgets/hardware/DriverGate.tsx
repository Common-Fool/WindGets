import { Bar } from "../../components/Bar";
import { Button } from "../../components/Button";
import { fileSize } from "../../lib/format";
import { useText } from "../../lib/i18n";
import {
  cancelPawnio,
  failureText,
  installPawnio,
  jobText,
  openPawnioPage,
  type DriverView,
} from "../../lib/pawnio";
import "../../components/ToolsGate.css";

/**
 * Слой поверх запертой карточки процессора: драйвера PawnIO нет, он устарел
 * или ждёт перезагрузки.
 *
 * Говорит прямо, что ставим и зачем, и даёт выбор: поставить кнопкой или
 * посмотреть страницу автора и решить самому. Размер — из Rust, до нажатия в
 * сеть не ходим. Отменить можно скачивание, а не сам установщик: брошенный
 * посреди установки драйвер хуже любого исхода.
 */
export function DriverGate({ driver }: { driver: DriverView }) {
  const t = useText();
  const job = driver.job;

  return (
    <div className="driver-gate">
      {job !== null ? (
        <>
          <p className="gate__title">{jobText(t, job)}</p>
          <div className="busy-bar">
            <Bar
              fill={job.stage === "downloading" && job.total ? job.received / job.total : null}
              thickness="thin"
            />
          </div>
          {(job.stage === "checking" || job.stage === "downloading") && (
            <Button variant="outline" onClick={() => void cancelPawnio()}>
              {t("common.cancel")}
            </Button>
          )}
        </>
      ) : driver.status === "restart" ? (
        <>
          <p className="gate__title">{t("pawnio.gate.restart")}</p>
          <p className="gate__line">{t("pawnio.gate.restartWhy")}</p>
        </>
      ) : (
        <>
          <p className="gate__title">
            {t(driver.status === "outdated" ? "pawnio.gate.outdated" : "pawnio.gate.missing")}
          </p>
          <p className="gate__line">{t("pawnio.gate.why")}</p>
          <Button onClick={() => void installPawnio().catch(() => {})}>
            {t(driver.status === "outdated" ? "pawnio.update" : "pawnio.install")}
          </Button>
          <button type="button" className="gate__link" onClick={() => openPawnioPage(driver)}>
            {t("pawnio.more")}
          </button>
          <p className="gate__hint tabular">{t("pawnio.source", { size: fileSize(driver.downloadBytes) })}</p>
        </>
      )}

      {job === null && driver.failure !== null && <p className="gate__error">{failureText(t, driver.failure)}</p>}
    </div>
  );
}
