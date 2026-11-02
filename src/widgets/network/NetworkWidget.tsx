import { useRef, type CSSProperties, type RefObject } from "react";
import { useTelemetry, type Reading as Sensor } from "../../lib/telemetry";
import { usePing, type Probe } from "../../lib/ping";
import { activeNetwork, type Network } from "../../lib/sensors";
import { formatReading, NO_VALUE } from "../../lib/format";
import type { Accent } from "../../lib/accent";
import { GlassCard } from "../../components/GlassCard";
import { StatusDot } from "../../components/StatusDot";
import { DownArrow, UpArrow } from "../../components/Arrows";
import { useLook } from "../../lib/look";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import { useContentHeight } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import "./network-reference.css";

/**
 * Пороги отклика: до сорока миллисекунд норма, до сотни терпимо, дальше связь
 * уже мешает работать. Прежних порогов шлюза здесь нет — шлюза нет тоже.
 */
const LATENCY = { good: 40, fair: 100 };

/**
 * Пороги разброса. Числа втрое строже: пять миллисекунд скачка между соседними
 * пакетами разговор ещё не портят, пятнадцать уже слышны.
 */
const JITTER = { good: 5, fair: 15 };

type Limits = { good: number; fair: number };

/**
 * Цвет по величине. Пустое значение красным не красим только там, где оно
 * означает «ещё не мерили», — это решает вызывающий.
 */
function quality(value: number | null, limits: Limits): Accent {
  if (value === null) return "danger";
  if (value <= limits.good) return "ok";
  if (value <= limits.fair) return "warn";
  return "danger";
}

/**
 * Миллисекунды в подпись. Меньше десяти — с десятой долей: у быстрого канала
 * и у джиттера весь разброс именно там, и округление до целых сделало бы
 * значение неподвижным.
 */
function ms(value: number | null | undefined): string | undefined {
  if (value == null) return undefined;
  return value < 10 ? value.toFixed(1) : value.toFixed(0);
}

/** Скорость разбираем на число и единицу: единица идёт отдельной подписью. */
function split(reading: Sensor | undefined): { value?: string; unit?: string } {
  if (!reading) return {};
  const text = formatReading(reading.value, reading.unit);
  if (text === NO_VALUE) return {};

  const gap = text.lastIndexOf(" ");
  return gap === -1 ? { value: text } : { value: text.slice(0, gap), unit: text.slice(gap + 1) };
}

/**
 * Скорость в столбце 180 — в одну строку всегда (1 ноября 2026, слово
 * пользователя): окно там стоит 180×180 без подгонки, и перенос единицы
 * растил бы его. От сотни десятые уходят — знаков не больше четырёх
 * («1023», «99.9»), а длинное число CSS ужимает шрифтом (`--net-ref-digits`).
 */
function narrowValue(value: string | undefined): string | undefined {
  if (value === undefined) return undefined;
  const number = Number(value);
  return Number.isFinite(number) && number >= 100 ? number.toFixed(0) : value;
}

/** Скорость и счётчик NEW; прежний Flow выше остаётся только у Classic. */
function ReferenceFlow({ direction, speed, total, live, narrow }: {
  direction: "down" | "up";
  speed: Sensor | undefined;
  total: Sensor | undefined;
  live: boolean;
  narrow: boolean;
}) {
  const t = useText();
  const parts = split(speed);
  const unit = parts.unit;
  const value = narrow ? narrowValue(parts.value) : parts.value;
  const down = direction === "down";
  return (
    <div className="net-ref__flow" data-direction={direction}>
      <div className="net-ref__flow-label">
        <span className="net-ref__arrow" data-accent={live ? (down ? "net-down" : "net-up") : "neutral"}>
          {down ? <DownArrow /> : <UpArrow />}
        </span>
        <span>{down ? "Download" : "Upload"}</span>
      </div>
      <div className="net-ref__speed tabular" style={{ "--net-ref-digits": Math.max(3, value?.length ?? 1) } as CSSProperties}>
        <span className="net-ref__speed-value">{value ?? NO_VALUE}</span>
        {unit !== undefined && <span className="net-ref__unit">{unit}</span>}
      </div>
      <span className="net-ref__total tabular" title={t("network.total")}>
        Σ {total ? formatReading(total.value, total.unit) : NO_VALUE}
      </span>
    </div>
  );
}

/** Цвет точек повторяет Target: неизвестность не равна неудачному ICMP. */
function ReferenceMeasurement({ probe, kind }: { probe: Probe; kind: "latency" | "jitter" }) {
  const reading = probe.reading;
  const latency = kind === "latency";
  const value = latency ? ms(reading?.latency) : reading?.jitter == null ? undefined : "±" + ms(reading.jitter);
  const accent: Accent = latency
    ? reading === null ? "neutral" : quality(reading.latency, LATENCY)
    : reading?.jitter == null ? "neutral" : quality(reading.jitter, JITTER);
  return (
    <span className={"net-ref__measurement net-ref__measurement--" + kind} data-accent={accent}>
      <span className="net-ref__dot" aria-hidden="true"><StatusDot /></span>
      <span className="net-ref__number tabular">{value ?? NO_VALUE}</span>
    </span>
  );
}

/** A: цели строками. Имя не заменяется потерями, оба показателя всегда видны. */
function ReferenceRows({ probes }: { probes: Probe[] }) {
  const t = useText();
  const hasLoss = probes.some((probe) => probe.reading !== null && probe.reading.loss > 0);
  return (
    <table className="net-ref__rows">
      <colgroup><col /><col className="net-ref__latency-col" /><col className="net-ref__jitter-col" /></colgroup>
      <thead><tr>
        <th scope="col">Ping{hasLoss && <span> · loss</span>}</th>
        <th scope="col">Latency <span className="net-ref__unit">ms</span></th>
        <th scope="col">Jitter <span className="net-ref__unit">ms</span></th>
      </tr></thead>
      <tbody>{probes.map((probe) => {
        const loss = probe.reading?.loss ?? 0;
        const percent = Math.round(loss * 100);
        return <tr key={probe.target}>
          <th scope="row" title={probe.target}>
            <span className="net-ref__target">
              <span className="net-ref__target-name">{probe.name}</span>
              {loss > 0 && <span className="net-ref__loss tabular" title={t("network.lossPercent", { percent })}>{percent}%</span>}
            </span>
          </th>
          <td><ReferenceMeasurement probe={probe} kind="latency" /></td>
          <td><ReferenceMeasurement probe={probe} kind="jitter" /></td>
        </tr>;
      })}</tbody>
    </table>
  );
}

/** B: цели колонками, сравнение отклика, разброса и потерь по строкам. */
function ReferenceMatrix({ probes }: { probes: Probe[] }) {
  const t = useText();
  return (
    <table className="net-ref__matrix">
      <colgroup><col className="net-ref__metric-col" />{probes.map((probe) => <col key={probe.target} />)}</colgroup>
      <thead><tr><th scope="col">Ping</th>{probes.map((probe) => <th scope="col" key={probe.target} title={probe.target}>{probe.name}</th>)}</tr></thead>
      <tbody>
        <tr><th scope="row">Latency<span className="net-ref__metric-unit">ms</span></th>{probes.map((probe) => <td key={probe.target}><ReferenceMeasurement probe={probe} kind="latency" /></td>)}</tr>
        <tr><th scope="row">Jitter<span className="net-ref__metric-unit">ms</span></th>{probes.map((probe) => <td key={probe.target}><ReferenceMeasurement probe={probe} kind="jitter" /></td>)}</tr>
        <tr><th scope="row">loss<span className="net-ref__metric-unit">%</span></th>{probes.map((probe) => {
          const percent = probe.reading === null ? undefined : Math.round(probe.reading.loss * 100);
          return <td className="net-ref__matrix-loss tabular" key={probe.target} title={percent === undefined ? undefined : t("network.lossPercent", { percent })}>{percent ?? NO_VALUE}</td>;
        })}</tr>
      </tbody>
    </table>
  );
}

function ReferenceNetwork({ network, probes, view, size, content }: {
  network: Network | undefined;
  probes: Probe[];
  view: number;
  size: number;
  content: RefObject<HTMLDivElement | null>;
}) {
  const t = useText();
  const matrix = view === 1;
  return (
    <div className={"net-ref " + (matrix ? "net-ref--matrix" : "net-ref--rows") + (!matrix || size === 1 ? " net-ref--narrow" : "")}
      ref={content} role="group" aria-label={t("network.reference.summary")}>
      <div className="net-ref__flows">
        <ReferenceFlow direction="down" speed={network?.download} total={network?.totalDownloaded} live={network !== undefined} narrow={!matrix || size === 1} />
        <ReferenceFlow direction="up" speed={network?.upload} total={network?.totalUploaded} live={network !== undefined} narrow={!matrix || size === 1} />
      </div>
      {matrix ? <ReferenceMatrix probes={probes} /> : <ReferenceRows probes={probes} />}
    </div>
  );
}

/**
 * Сеть и отклик одной карточкой.
 *
 * Прежде это были два окна на пять чисел: у каждого показателя своя строка с
 * подписью, своя крупная цифра, своя единица под ней и своя подпись под всем
 * этим. Здесь показатель — это одна строка, а вторая занята тем, что под ней
 * подписано.
 *
 * Источники у половин разные, и при отказе LHM это видно: скорости становятся
 * прочерками, а стрелки гаснут в серый — пинг при этом продолжает работать.
 * Полосы «LibreHardwareMonitor не отвечает» тут нет: в карточке высотой полтора
 * десятка строк она съела бы пятую часть, а сказала бы то же самое.
 */
export function NetworkWidget() {
  const telemetry = useTelemetry();
  const { probes } = usePing();
  const design = useLook().effectiveDesign;
  const view = useWidgetView();
  const size = useWidgetSize();
  const reference = design === "reference";
  const content = useRef<HTMLDivElement>(null);
  // Classic высоту не меняет; NEW измеряет всю новую карточку, включая поля.
  // «Потоки и строки» NEW — окно 180×180 без подгонки (1 ноября 2026, слово
  // пользователя): скорость там в одну строку, и высота не меняется
  useContentHeight(content, reference && view === 1 && probes.length > 0, [design, view, size].join("|"));

  // Адаптер выбирается по трафику: имена локализованы, виртуальных несколько
  const network = activeNetwork(telemetry);

  return (
    <div className="widget">
      <GlassCard accent="neutral" flush>
        {/* Classic — та же композиция «Потоков и строк» (2 ноября 2026, слово
            пользователя): окно и там квадрат 180, а прежняя Classic-карточка
            была снята под полосу высотой в сотню точек. Цвета и шрифты —
            дизайна, вид у Classic один (`design_view`) */}
        <ReferenceNetwork network={network} probes={probes} view={reference ? view : 0} size={size} content={content} />
      </GlassCard>
    </div>
  );
}
