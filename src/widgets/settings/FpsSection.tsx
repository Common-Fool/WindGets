import { ComponentsField } from "./ToolsFields";
import { useText } from "../../lib/i18n";

/** Настройки счётчика кадров: PresentMon — скачать, обновить, удалить. */
export function FpsSection() {
  const t = useText();

  return (
    <div className="rows">
      <ComponentsField module="fps" extra={t("fps.settings.note")} />
    </div>
  );
}
