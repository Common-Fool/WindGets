import { fileSize } from "../lib/format";
import {
  cancelTools,
  failureText,
  installTools,
  listOf,
  missingFor,
  nameOf,
  useTools,
  type ModuleId,
  type ToolsView,
} from "../lib/tools";
import { useText, type Text } from "../lib/i18n";
import { Bar } from "./Bar";
import { Button } from "./Button";
import "./ToolsGate.css";

/**
 * Согласие на скачивание программ модуля — вместо карточки, пока их нет.
 *
 * До нажатия приложение в сеть не ходит: размеры здесь из каталога, а не от
 * GitHub. Нажатие ставит всё, чего не хватает, по очереди, с прогрессом и
 * отменой. Зачем программы нужны, говорит своя строка у каждого модуля.
 */
export function ToolsGate({ module }: { module: ModuleId }) {
  const tools = useTools();
  const t = useText();
  const missing = missingFor(tools, module);

  if (tools.job !== null) {
    return (
      <div className="gate">
        <p className="gate__title">{stageText(t, tools)}</p>
        <div className="busy-bar">
          <Bar
            fill={tools.job.total ? tools.job.received / tools.job.total : null}
            thickness="thin"
          />
        </div>
        <Button variant="outline" onClick={() => void cancelTools()}>
          {t("common.cancel")}
        </Button>
      </div>
    );
  }

  // Системный ffmpeg ещё ищется — просить скачать его было бы рано
  if (tools.probing) {
    return (
      <div className="gate">
        <p className="gate__hint">{t("tools.gate.probing")}</p>
      </div>
    );
  }

  const download = missing.reduce((sum, component) => sum + component.downloadBytes, 0);
  const disk = missing.reduce((sum, component) => sum + component.diskBytes, 0);

  function install() {
    // Отказ «занято» значит, что установка уже идёт, — её прогресс и так
    // окажется на экране следующим событием
    installTools(module).catch(() => {});
  }

  return (
    <div className="gate">
      <p className="gate__title">
        {t(`tools.gate.needs.${module}`, { list: listOf(t, missing.map((component) => component.name)) })}
      </p>
      <p className="gate__line tabular">
        {t("tools.gate.sizes", { download: fileSize(download), disk: fileSize(disk) })}
      </p>
      <Button onClick={install}>
        {t("tools.gate.download")}
      </Button>
      {tools.failure !== null && (
        <p className="gate__error">
          {nameOf(tools, tools.failure.component)}: {failureText(t, tools.failure.code)}
        </p>
      )}
      <p className="gate__hint">{t("tools.gate.source")}</p>
    </div>
  );
}

function stageText(t: Text, tools: ToolsView): string {
  const job = tools.job;
  if (job === null) return "";
  const name = nameOf(tools, job.component);

  switch (job.stage) {
    case "checking":
      return t("tools.stage.checking", { name });
    case "unpacking":
      return t("tools.stage.unpacking", { name });
    case "downloading":
      return job.total
        ? t("tools.stage.downloadingOf", {
            name,
            received: fileSize(job.received),
            total: fileSize(job.total),
          })
        : t("tools.stage.downloading", { name, received: fileSize(job.received) });
  }
}
