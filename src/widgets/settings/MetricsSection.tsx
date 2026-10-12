import { Check } from "../../components/Check";
import { ChevronIcon } from "../../components/Icons";
import { showReading } from "../../lib/format";
import { useText, type Key } from "../../lib/i18n";
import {
  boardTemp,
  chassisFan,
  cpuClock,
  cpuFan,
  cpuPower,
  cpuTemp,
  cpuVoltage,
  gpuClock,
  gpuFans,
  gpuMemoryClock,
  gpuPower,
  gpuSecondTemp,
  gpuTemp,
  gpuVoltage,
} from "../../lib/sensors";
import {
  METRICS,
  PICK_OFF,
  setMetricHidden,
  setSensorPick,
  useSettings,
  type Metric,
  type SensorSlot,
} from "../../lib/settings";
import { findReading, useTelemetry, type Choices, type SourceState } from "../../lib/telemetry";

const METRIC_NAMES: Record<Metric, Key> = {
  temperature: "metrics.temperature",
  fan: "metrics.fan",
  clock: "metrics.clock",
  power: "metrics.power",
  voltage: "metrics.voltage",
};

/** Есть ли ряд на этом железе — хоть одна его ячейка. */
function available(id: string, state: SourceState): Record<Metric, boolean> {
  if (id === "gpu") {
    return {
      temperature: gpuTemp(state) !== undefined || gpuSecondTemp(state) !== undefined,
      fan: gpuFans(state).length > 0,
      clock: gpuClock(state) !== undefined || gpuMemoryClock(state) !== undefined,
      power: gpuPower(state) !== undefined,
      voltage: gpuVoltage(state) !== undefined,
    };
  }
  return {
    temperature: cpuTemp(state) !== undefined || boardTemp(state) !== undefined,
    fan: cpuFan(state) !== undefined || chassisFan(state) !== undefined,
    clock: cpuClock(state) !== undefined,
    power: cpuPower(state) !== undefined,
    voltage: cpuVoltage(state) !== undefined,
  };
}

/**
 * Состав карточки процессора или видеокарты (этап 58, слово пользователя):
 * какие ряды показывать. Снятый ряд уходит из карточки, и окно становится
 * ниже — его высота следует из содержимого. Загрузки (кольца или полосы) не
 * снимаются никогда.
 *
 * Потребление и напряжение — две галочки одного ряда Power: ватты и вольты
 * снимаются порознь. Ряда нет на этом железе — галочка погашена и говорит
 * об этом строкой; пока датчики спят, судить не по чему, и погашенных нет.
 *
 * У процессора ниже — ручной выбор датчиков платы: на незнакомой плате
 * вентиляторы зовутся «Fan #1…», и угадать процессорный автоматика не может.
 */
export function MetricsSection({ id }: { id: string }) {
  const t = useText();
  const telemetry = useTelemetry();
  const hidden = useSettings().hiddenMetrics[id] ?? [];
  const live = telemetry.status === "ok";
  const here = available(id, telemetry);
  const gpu = live && id === "gpu" ? telemetry.snapshot.roles.gpu : null;

  return (
    <div className="rows">
      <div className="field">
        {/* Подсказка на заголовке: над галочками она мешала бы */}
        <div className="field__head" data-hint={t("metrics.hint")}>
          <span className="field__name">{t("metrics.title")}</span>
        </div>

        {/* Из нескольких дискретных показывается одна — видно, какая */}
        {gpu !== null && <p className="note">{t("metrics.gpuName", { name: gpu.name })}</p>}

        <div className="rows rows--tight">
          {METRICS.map((metric) => {
            const absent = live && !here[metric];
            return (
              <label key={metric} className="switch">
                <Check
                  checked={!hidden.includes(metric)}
                  label={t(METRIC_NAMES[metric])}
                  quiet
                  disabled={absent}
                  onChange={(next) => setMetricHidden(id, metric, !next)}
                />
                <span className="switch__text">
                  <span className="switch__name">{t(METRIC_NAMES[metric])}</span>
                  {absent && <span className="switch__hint">{t("metrics.absent")}</span>}
                </span>
              </label>
            );
          })}
        </div>
      </div>

      {id === "cpu" && <BoardPicks />}
    </div>
  );
}

/** Место выбора, его подпись и из чего выбирают. */
const SLOTS: { slot: SensorSlot; name: Key; from: (choices: Choices) => string[] }[] = [
  { slot: "cpuFan", name: "metrics.cpuFan", from: (choices) => choices.fans },
  { slot: "chassisFan", name: "metrics.chassisFan", from: (choices) => choices.fans },
  { slot: "boardTemp", name: "metrics.boardTemp", from: (choices) => choices.temperatures },
  { slot: "cpuVoltage", name: "metrics.cpuVoltage", from: (choices) => choices.voltages },
];

/** Путь без имени машины — так выбор хранится, как и в Rust. */
function withoutMachine(path: string): string {
  return path.slice(path.indexOf("/") + 1);
}

/**
 * Ручной выбор датчиков платы: четыре раскрывающихся списка, как выбор языка.
 *
 * «Автоматически» — то, что нашла автоматика по имени; «Не показывать» —
 * ячейка уходит из карточки. Дальше датчики подходящего вида с живым
 * значением: по нему и узнают, какой «Fan #3» крутит процессорный кулер.
 */
function BoardPicks() {
  const t = useText();
  const telemetry = useTelemetry();
  const picks = useSettings().sensorPicks;
  const choices = telemetry.status === "ok" ? telemetry.snapshot.choices : null;

  return (
    <div className="field">
      <div className="field__head" data-hint={t("metrics.boardHint")}>
        <span className="field__name">{t("metrics.board")}</span>
      </div>

      {choices === null ? (
        <p className="note">{t("metrics.waiting")}</p>
      ) : (
        <div className="rows rows--tight">
          {SLOTS.map(({ slot, name, from }) => (
            <PickRow
              key={slot}
              slot={slot}
              title={t(name)}
              paths={from(choices)}
              pick={picks[slot] ?? ""}
              state={telemetry}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function PickRow({
  slot,
  title,
  paths,
  pick,
  state,
}: {
  slot: SensorSlot;
  title: string;
  paths: string[];
  pick: string;
  state: SourceState;
}) {
  const t = useText();

  const options = paths.map((path) => {
    const reading = findReading(state, path);
    const name = reading?.name ?? path.slice(path.lastIndexOf("/") + 1);
    return { value: withoutMachine(path), label: `${name} · ${showReading(reading) ?? "—"}` };
  });
  // Одинаковые имена у разных чипов различает чип: «Temperature #1» бывает и
  // у Super I/O, и у контроллера
  const counts = new Map<string, number>();
  for (const option of options) {
    const name = option.label.split(" · ")[0]!;
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  for (const option of options) {
    const name = option.label.split(" · ")[0]!;
    if ((counts.get(name) ?? 0) > 1) {
      const parts = option.value.split("/");
      option.label = `${option.label} (${parts[parts.length - 3] ?? parts[0]})`;
    }
  }

  // Выбранный датчик пропал (сменили плату) — пункт остаётся, иначе список
  // молча показал бы «Автоматически», а в файле стоял бы старый выбор
  const known = pick === "" || pick === PICK_OFF || options.some((option) => option.value === pick);

  return (
    <div className="field__head field__head--center">
      <label className="field__name" htmlFor={`pick-${slot}`}>{title}</label>
      <span className="select">
        <select
          id={`pick-${slot}`}
          className="select__control"
          value={pick}
          onChange={(event) => setSensorPick(slot, event.target.value === "" ? null : event.target.value)}
        >
          <option value="">{t("metrics.auto")}</option>
          <option value={PICK_OFF}>{t("metrics.off")}</option>
          {!known && (
            <option value={pick}>{t("metrics.missing", { name: pick.slice(pick.lastIndexOf("/") + 1) })}</option>
          )}
          {options.map((option) => (
            <option key={option.value} value={option.value}>{option.label}</option>
          ))}
        </select>
        <span className="select__chevron" aria-hidden>
          <ChevronIcon />
        </span>
      </span>
    </div>
  );
}
