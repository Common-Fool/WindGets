import { useText } from "../lib/i18n";
import "./Undo.css";

/**
 * Кнопка возврата посреди карточки.
 *
 * Стоит в центре, а не строкой внизу: там она была бы ещё одной подписью среди
 * подписей, а тут её видно сразу — на месте того, что только что исчезло.
 *
 * ВАЖНО: **обёртка накрывает карточку целиком, но мышь ловит одна кнопка.**
 * Иначе содержимое под возвратом перестало бы нажиматься на все восемь секунд —
 * в списке дел это значит, что галочку не поставить.
 */
export function Undo({ onClick }: { onClick: () => void }) {
  const t = useText();
  return (
    <div className="undo">
      <button type="button" className="undo__button" onClick={onClick}>
        {t("common.undo")}
      </button>
    </div>
  );
}
