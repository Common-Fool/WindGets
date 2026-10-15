import { PlusIcon } from "./Icons";
import { useText } from "../lib/i18n";
import "./Dots.css";

/**
 * Страницы карточки точками.
 *
 * Одна на заметки и на список дел: у обоих карточка держит не одну вещь, а
 * несколько, и переключают их одинаково. Имён у страниц нет — точка и есть имя:
 * страниц не больше горсти, и подписи к ним заняли бы строку, которой в этих
 * карточках и так не хватает.
 *
 * Точка — кнопка, и это осознанно: за неё окно не тянется, зато в неё попадают
 * мышью. Пустое место строки при этом остаётся ручкой окна.
 *
 * Почта листает ими ящики (этап 53): там у точки есть имя — адрес ящика, — и
 * оно всплывает подсказкой, а «+» в карточке нет: ящики заводят в настройках.
 */
type Props = {
  className?: string;
  /** Сколько страниц. Ноль и один — точку всё равно рисуем. */
  count: number;
  /** Какая открыта. */
  active: number;
  /** Дальше этого не заводим: точки должны помещаться в узкое окно. */
  limit: number;
  onPick: (at: number) => void;
  /** Нет — нет и «+». */
  onAdd?: () => void;
  /** Подсказки точек по порядку. Нет — «Страница N». */
  titles?: string[];
  /** Непрочитанная книга не допускает изменения страниц. */
  disabled?: boolean;
  /** Прокручиваемая полоса GPT удерживает мышь у своего бегунка. */
  scrollable?: boolean;
};

export function Dots({ count, active, limit, onPick, onAdd, titles, disabled = false, scrollable = false, className }: Props) {
  const t = useText();
  // Пустая книга рисуется как одна страница: пустая заметка — это страница,
  // которую ещё не написали, а не отсутствие страниц
  const shown = Math.max(count, 1);

  return (
    <div className={["dots", className].filter(Boolean).join(" ")} data-no-drag={scrollable || undefined}>
      {Array.from({ length: shown }, (_, at) => (
        <button
          key={at}
          type="button"
          className="dots__dot"
          data-active={at === active}
          aria-current={at === active ? "page" : undefined}
          disabled={disabled}
          title={titles?.[at] ?? t("pages.page", { number: at + 1 })}
          onClick={() => onPick(at)}
        />
      ))}

      {onAdd !== undefined && shown < limit && (
        <button type="button" className="dots__add" title={t("pages.add")} disabled={disabled} onClick={onAdd}>
          <PlusIcon />
        </button>
      )}
    </div>
  );
}
