import { useMemo, useState } from "react";
import { useTelemetry, type Reading, type SourceState } from "../../lib/telemetry";
import { formatReading, NO_VALUE } from "../../lib/format";
import "./debug.css";

function readingsOf(state: SourceState): Reading[] {
  return state.status === "ok" ? state.snapshot.readings : [];
}

function StatusLine({ state, shown }: { state: SourceState; shown: number }) {
  // «Не нужны» — миг после включения отладки, пока опрос не проснулся
  if (state.status === "connecting" || state.status === "idle") {
    return <span className="debug__status">Подключение к LibreHardwareMonitor…</span>;
  }

  if (state.status === "unavailable") {
    return (
      <span className="debug__status debug__status--error">
        LibreHardwareMonitor не отвечает: {state.reason}
      </span>
    );
  }

  const at = new Date(state.snapshot.at).toLocaleTimeString("ru-RU");
  const total = state.snapshot.readings.length;
  const count = shown === total ? `${total}` : `${shown} из ${total}`;
  return (
    <span className="debug__status">
      Датчиков: {count} · обновлено в {at}
    </span>
  );
}

export function DebugWidget() {
  const state = useTelemetry();
  const [filter, setFilter] = useState("");

  const readings = readingsOf(state);

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return readings;
    return readings.filter((reading) => reading.path.toLowerCase().includes(needle));
  }, [readings, filter]);

  return (
    <div className="debug">
      <div className="debug__bar">
        <StatusLine state={state} shown={visible.length} />
        <input
          className="debug__filter"
          type="search"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Фильтр по пути: gpu, tctl, nvme…"
          spellCheck={false}
        />
      </div>

      <div className="debug__scroll">
        {visible.length === 0 ? (
          <p className="debug__empty">
            {state.status === "ok" ? "Ничего не нашлось по фильтру." : "Данных пока нет."}
          </p>
        ) : (
          <table className="debug__table">
            <thead>
              <tr>
                <th>Путь</th>
                <th>Значение</th>
                <th>Тип</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((reading) => {
                const text = formatReading(reading.value, reading.unit);
                return (
                  <tr key={reading.path}>
                    <td className="debug__path">{reading.path}</td>
                    <td
                      className={
                        text === NO_VALUE ? "debug__value debug__value--empty" : "debug__value"
                      }
                    >
                      {text}
                    </td>
                    <td className="debug__kind">{reading.kind}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
