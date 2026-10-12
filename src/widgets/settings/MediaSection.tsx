import { useState } from "react";
import { Check } from "../../components/Check";
import { setSource, useMedia } from "../../lib/media";
import { useText } from "../../lib/i18n";

/**
 * Настройки виджета «Медиа»: выбор видимых источников воспроизведения.
 *
 * Список формируется автоматически по мере появления медиасессий в системе.
 * Снятая галочка скрывает источник в карточке виджета, не удаляя его из списка.
 */
export function MediaSection() {
  const { sources } = useMedia();
  const t = useText();
  const [error, setError] = useState<string | null>(null);

  async function toggle(id: string, enabled: boolean) {
    setError(null);
    try {
      await setSource(id, enabled);
    } catch (reason) {
      setError(String(reason));
    }
  }

  return (
    <div className="rows">
      <div className="field">
        {/* Подсказка на заголовке: над галочками источников она мешала бы */}
        <div className="field__head" data-hint={t("media.settings.sourcesHint")}>
          <span className="field__name">{t("media.settings.sources")}</span>
          <span className="field__value">{sources.length}</span>
        </div>

        {sources.length === 0 ? (
          <p className="note">{t("media.settings.none")}</p>
        ) : (
          <div className="rows rows--tight">
            {sources.map((source) => {
              const name =
                source.title.trim() !== "" ? source.title : source.id;
              return (
                <label key={source.id} className="switch">
                  <Check
                    checked={source.enabled}
                    label={name}
                    onChange={(next) => void toggle(source.id, next)}
                  />
                  <span className="switch__text">
                    <span className="switch__name">{name}</span>
                    <span className="switch__hint">{source.id}</span>
                  </span>
                </label>
              );
            })}
          </div>
        )}

        {error !== null && <p className="note note--error">{error}</p>}
      </div>
    </div>
  );
}
