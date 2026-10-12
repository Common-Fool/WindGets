import type { Accent } from "../lib/accent";
import "./Check.css";

type Props = {
  checked: boolean;
  /** Смешанное состояние: выбрано у части. Рисует черту вместо птички. */
  mixed?: boolean;
  /** Название для тех, кто галочку не видит: в таблице подписи нет. */
  label: string;
  /** Настройка есть, но применять её сейчас не к чему. */
  disabled?: boolean;
  /** Не задан — берётся акцент родителя. */
  accent?: Accent;
  /**
   * Без всплывающего имени: подпись и так стоит рядом, а пояснение к строке
   * показывает подсказка окна настроек — две всплывашки разом спорили бы.
   */
  quiet?: boolean;
  onChange: (next: boolean) => void;
};

/**
 * Галочка для таблицы настроек.
 *
 * `Toggle` тут не годится: он пилюля с подписью, а в столбце подпись стоит
 * один раз в шапке — десять пилюль подряд читались бы как десять кнопок с
 * одинаковым именем.
 */
export function Check({
  checked,
  mixed = false,
  label,
  disabled = false,
  accent,
  quiet = false,
  onChange,
}: Props) {
  return (
    <button
      type="button"
      className="check"
      data-accent={accent}
      role="checkbox"
      aria-checked={mixed ? "mixed" : checked}
      aria-label={label}
      title={quiet ? undefined : label}
      disabled={disabled}
      onClick={() => onChange(mixed ? true : !checked)}
    >
      <svg className="check__mark" viewBox="0 0 12 12" aria-hidden="true">
        {mixed ? (
          <path d="M3 6h6" />
        ) : (
          <path d="M2.5 6.4 4.8 8.7 9.5 3.6" />
        )}
      </svg>
    </button>
  );
}
