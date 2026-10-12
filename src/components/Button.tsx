import type { ReactNode } from "react";
import "./Button.css";

type Props = {
  /** Без подписи кнопка становится квадратной, и подпись уходит в `label`. */
  children?: ReactNode;
  icon?: ReactNode;
  variant?: "solid" | "outline";
  /** Замер идёт: кнопка недоступна, иконка крутится. */
  busy?: boolean;
  disabled?: boolean;
  /** Название для тех, кто кнопку не видит. Обязателен, если нет подписи. */
  label?: string;
  onClick?: () => void;
};

export function Button({
  children,
  icon,
  variant = "solid",
  busy = false,
  disabled = false,
  label,
  onClick,
}: Props) {
  const classes = [
    "btn",
    `btn--${variant}`,
    busy ? "btn--busy" : "",
    children === undefined ? "btn--icon" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      type="button"
      className={classes}
      disabled={disabled || busy}
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      {icon !== undefined && <span className="btn__icon">{icon}</span>}
      {/* Подпись отдельным элементом: в узкой карточке она гаснет, и целиться
          порогам нужно во что-то, а не в голый текст рядом со значком */}
      {children !== undefined && <span className="btn__label">{children}</span>}
    </button>
  );
}
