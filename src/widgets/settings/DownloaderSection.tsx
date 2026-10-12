import { useDownloader } from "../../lib/downloader";
import { ComponentsField, OutputFolderField } from "./ToolsFields";
import { useText } from "../../lib/i18n";

/** Настройки загрузчика: папка вывода и yt-dlp с его движком и ffmpeg. */
export function DownloaderSection() {
  const view = useDownloader();
  const t = useText();

  return (
    <div className="rows">
      <OutputFolderField module="downloader" dir={view.outDir} custom={view.outDirCustom} />
      <ComponentsField module="downloader" extra={t("downloader.settings.note")} />
    </div>
  );
}
