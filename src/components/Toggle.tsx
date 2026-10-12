import type { Accent } from "../lib/accent";
import "./Toggle.css";

type Props = {
  label: string;
  /**
   * Подпись для тесной раскладки: «Mid» вместо «Balanced». Рисуются обе, а
   * какую видно, решает CSS карточки порогом `@container` — сам чип о
   * пороге ничего не знает. Без неё чип остаётся как был.
   */
  short?: string;
  active: boolean;
  /** Не задан — берётся акцент родителя. */
  accent?: Accent;
  onChange: (next: boolean) => void;
};

export function Toggle({ label, short, active, accent, onChange }: Props) {
  return (
    <button
      type="button"
      className="toggle"
      data-accent={accent}
      aria-pressed={active}
      aria-label={short === undefined ? undefined : label}
      onClick={() => onChange(!active)}
    >
      <span className="toggle__dot" />
      {short === undefined ? (
        label
      ) : (
        <>
          <span className="toggle__long">{label}</span>
          <span className="toggle__short">{short}</span>
        </>
      )}
    </button>
  );
}
