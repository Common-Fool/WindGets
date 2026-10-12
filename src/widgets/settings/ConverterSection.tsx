import { useConverter } from "../../lib/converter";
import { ComponentsField, OutputFolderField } from "./ToolsFields";

/** Настройки конвертера: папка вывода и ffmpeg. */
export function ConverterSection() {
  const view = useConverter();

  return (
    <div className="rows">
      <OutputFolderField module="converter" dir={view.outDir} custom={view.outDirCustom} />
      <ComponentsField module="converter" />
    </div>
  );
}
