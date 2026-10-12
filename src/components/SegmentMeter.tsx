import type { Accent } from "../lib/accent";
import "./SegmentMeter.css";

type Props = {
  /** Сколько сегментов закрашено. */
  level: number;
  total?: number;
  /**
   * Цвет задаёт вызывающий по смыслу: danger, warn или ok. Радуги внутри
   * компонента нет — палитра одна на весь проект.
   */
  accent?: Accent;
};

export function SegmentMeter({ level, total = 4, accent }: Props) {
  return (
    <div className="segments" data-accent={accent}>
      {Array.from({ length: total }, (_, index) => (
        <span
          key={index}
          className={index < level ? "segments__item segments__item--filled" : "segments__item"}
        />
      ))}
    </div>
  );
}
