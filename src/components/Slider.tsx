import type { CSSProperties } from "react";
import "./Slider.css";

type Props = {
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  ariaLabel?: string;
  vertical?: boolean;
  onChange: (value: number) => void;
  /**
   * Ползунок отпустили.
   *
   * Нужен там, где каждое движение показывают начерно, а запоминают один раз:
   * запись на диск на каждый пиксель стоила бы дороже самой отрисовки.
   */
  onCommit?: () => void;
  /**
   * Плоский вид: полоса во всю ширину и без бегунка.
   *
   * Для регулятора, вделанного в плитку или нижний край пульта.
   */
  flat?: boolean;
};

export function Slider({ value, min, max, step = 1, disabled = false, ariaLabel, vertical = false, onChange, onCommit, flat = false }: Props) {
  const filled = max > min ? ((value - min) / (max - min)) * 100 : 0;
  // Заполнение уезжает в CSS переменной: градиент трека считается по ней
  const style = { "--slider-fill": `${filled.toFixed(2)}%` } as CSSProperties;

  return (
    <input
      type="range"
      className={flat ? "slider slider--flat" : "slider"}
      style={style}
      value={value}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      aria-label={ariaLabel}
      aria-orientation={vertical ? "vertical" : undefined}
      data-vertical={vertical || undefined}
      onChange={(event) => onChange(Number(event.target.value))}
      // Мышью и клавишами ползунок двигают порознь, отпускают тоже
      onPointerUp={onCommit}
      onKeyUp={onCommit}
      onBlur={onCommit}
    />
  );
}
