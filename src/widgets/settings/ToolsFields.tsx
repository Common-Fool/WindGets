import { useState } from "react";
import { Button } from "../../components/Button";
import { fileSize } from "../../lib/format";
import { openOutputDir, pickOutputDir, resetOutputDir } from "../../lib/output";
import { useSettings } from "../../lib/settings";
import {
  cancelTools,
  componentsFor,
  failureText,
  installTools,
  listOf,
  nameOf,
  otherUsers,
  removeTools,
  updateTool,
  useTools,
  type ModuleId,
  type ToolComponent,
} from "../../lib/tools";
import { useText, type Text } from "../../lib/i18n";

/*
 * Общие поля настроек конвертера и загрузчика: папка вывода и докачиваемые
 * программы.
 */

/** Куда кладётся готовое. */
export function OutputFolderField({
  module,
  dir,
  custom,
}: {
  module: ModuleId;
  dir: string | null;
  custom: boolean;
}) {
  const t = useText();
  const [openFailed, setOpenFailed] = useState(false);
  return (
    <div className="field" data-hint={t("tools.folderNote")}>
      <div className="field__head">
        <span className="field__name">{t("tools.folder")}</span>
        <span className="field__value">{t(custom ? "tools.folderCustom" : "tools.folderDefault")}</span>
      </div>
      <p className="note" data-no-drag>
        {dir ?? t("tools.noDesktop")}
      </p>
      <div className="choices">
        <Button onClick={() => void pickOutputDir(module)}>{t("tools.change")}</Button>
        <Button
          variant="outline"
          onClick={() => {
            setOpenFailed(false);
            openOutputDir(module).catch(() => setOpenFailed(true));
          }}
        >
          {t("tools.open")}
        </Button>
        {custom && (
          <Button variant="outline" onClick={() => void resetOutputDir(module)}>
            {t("tools.reset")}
          </Button>
        )}
      </div>
      {/* Своя папка на отключённом сетевом диске: молчание выглядело бы как
          неработающая кнопка */}
      {openFailed && <p className="note note--error" role="alert">{t("tools.openFailed")}</p>}
    </div>
  );
}

/**
 * Программы модуля: что стоит, какая версия, проверка обновлений, удаление.
 *
 * Удаляется только скачанная — системный ffmpeg ставил человек, и кнопки у
 * него нет. Подтверждения нет тоже: удалённое скачивается заново той же
 * кнопкой, данных тут не теряется. Нужна программа другому включённому модулю
 * (ffmpeg у конвертера и загрузчика общий) — под строкой это сказано прямо.
 *
 * Откуда программы и как проверяются — подсказкой на заголовке, а не абзацем:
 * `extra` дописывает к ней своё у модуля (у загрузчика — как yt-dlp
 * обновляется сам).
 */
export function ComponentsField({ module, extra }: { module: ModuleId; extra?: string }) {
  const tools = useTools();
  const needs = tools.modules.find((entry) => entry.id === module)?.needs ?? [];
  const settings = useSettings();
  const components = componentsFor(tools, module);
  const missing = components.some((component) => component.presence === "missing");
  const busy = tools.job !== null;
  const t = useText();

  /** Включённые модули, которым эта программа нужна тоже, — их названиями. */
  const sharedWith = (component: ToolComponent): string[] =>
    otherUsers(tools, component.id, module)
      .map((id) => settings.widgets.find((row) => row.id === id))
      .filter((row) => row !== undefined && row.enabled)
      .map((row) => row!.title);

  return (
    <div className="field">
      {/* Подсказка на заголовке, а не на всём поле: над кнопками строк она
          всплывала бы, пока в них целятся */}
      <span className="field__name" data-hint={extra === undefined ? t("tools.note") : `${t("tools.note")}\n${extra}`}>
        {t("tools.programs")}
      </span>
      <div className="rows rows--tight">
        {components.map((component) => {
          const shared = component.presence === "installed" ? sharedWith(component) : [];
          return (
            <div key={component.id} className="field__head">
              <span className="switch__text">
                <span className="switch__name">{component.name}</span>
                <span className="switch__hint">{describe(t, component)}</span>
                {shared.length > 0 && (
                  <span className="switch__hint">{t("tools.alsoNeeded", { list: listOf(t, shared) })}</span>
                )}
              </span>
              {component.presence === "installed" && (
                <span className="choices">
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => void updateTool(component.id).catch(() => {})}
                  >
                    {t("tools.checkUpdates")}
                  </Button>
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => void removeTools([component.id]).catch(() => {})}
                  >
                    {t("tools.remove")}
                  </Button>
                </span>
              )}
            </div>
          );
        })}
      </div>

      {busy && tools.job !== null && (
        <div className="choices">
          <p className="note">
            {nameOf(tools, tools.job.component)}:{" "}
            {tools.job.stage === "checking"
              ? t("tools.job.checking")
              : tools.job.stage === "unpacking"
                ? t("tools.job.unpacking")
                : tools.job.total
                  ? t("tools.job.receivedOf", {
                      received: fileSize(tools.job.received),
                      total: fileSize(tools.job.total),
                    })
                  : t("tools.job.received", { received: fileSize(tools.job.received) })}
          </p>
          <Button variant="outline" onClick={() => void cancelTools()}>
            {t("common.cancel")}
          </Button>
        </div>
      )}

      {/* Пока ищется системный ffmpeg, «не хватает» — ещё не ответ: нажатие
          скачало бы то, что, возможно, уже стоит */}
      {!busy && missing && !tools.probing && (
        <Button onClick={() => void installTools(module).catch(() => {})}>{t("tools.installMissing")}</Button>
      )}

      {tools.failure !== null && needs.includes(tools.failure.component) && (
        <p className="note note--error">
          {nameOf(tools, tools.failure.component)}: {failureText(t, tools.failure.code)}
        </p>
      )}
      {tools.notice !== null && needs.includes(tools.notice.component) && (
        <p className="note">
          {nameOf(tools, tools.notice.component)}:{" "}
          {t(tools.notice.code === "updated" ? "tools.updated" : "tools.upToDate", {
            version: tools.notice.version,
          })}
        </p>
      )}
    </div>
  );
}

function describe(t: Text, component: ToolComponent): string {
  switch (component.presence) {
    case "installed":
      return component.version !== null
        ? t("tools.installedVersion", { version: component.version })
        : t("tools.installed");
    case "system":
      return t("tools.system");
    case "missing":
      return t("tools.missing", {
        download: fileSize(component.downloadBytes),
        disk: fileSize(component.diskBytes),
      });
  }
}
