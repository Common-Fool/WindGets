import "./Bar.css";

type Props = {
  /** Доля заполнения от 0 до 1. `null` — данных нет, остаётся пустой трек. */
  fill: number | null;
  thickness?: "thick" | "thin";
};

export function Bar({ fill, thickness = "thick" }: Props) {
  const clamped = fill === null ? null : Math.min(Math.max(fill, 0), 1);

  return (
    <div className={thickness === "thin" ? "bar bar--thin" : "bar"}>
      {clamped !== null && (
        <div className="bar__fill" style={{ width: `${(clamped * 100).toFixed(2)}%` }} />
      )}
    </div>
  );
}
