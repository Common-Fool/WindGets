import { useRef, type ReactNode, type RefObject } from "react";
import { useTelemetry, type Reading as Sensor } from "../../lib/telemetry";
import { usePing, type Probe } from "../../lib/ping";
import { activeNetwork, type Network } from "../../lib/sensors";
import { formatReading, NO_VALUE } from "../../lib/format";
import type { Accent } from "../../lib/accent";
import { GlassCard } from "../../components/GlassCard";
import { BigStat } from "../../components/BigStat";
import { StatusDot } from "../../components/StatusDot";
import { DownArrow, UpArrow } from "../../components/Arrows";
import { useLook } from "../../lib/look";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import { useContentHeight } from "../../lib/grow";
import { useText } from "../../lib/i18n";
import "./network.css";
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
 * Скорость в одну строку: стрелка, число, единица. Под ней итог за сессию.
 *
 * Слова «Приём» и «Отдача» ушли вместе со своей строкой — стрелка и цвет
 * говорят то же самое, не занимая высоты.
 */
function Flow({
  accent,
  arrow,
  speed,
  total,
}: {
  accent: Accent;
  arrow: ReactNode;
  speed: Sensor | undefined;
  total: Sensor | undefined;
}) {
  const { value, unit } = split(speed);

  return (
    <div className="net__cell" data-accent={accent}>
      <div className="net__line">
        <span className="net__arrow">{arrow}</span>
        <BigStat size="sm" value={value} unit={unit} />
      </div>
      {/* Слова «всего» нет: в строке под скоростью другого числа не бывает */}
      <span className="net__hint tabular">
        {total ? formatReading(total.value, total.unit) : NO_VALUE}
      </span>
    </div>
  );
}

/**
 * Цель пинга: отклик и разброс, каждый со своей точкой состояния.
 *
 * Под ними имя оператора, а не адрес: «208.67.222.222» вдвое длиннее соседей
 * и разорвал бы колонки. Адрес остаётся подсказкой — он нужен раз в год.
 *
 * Потери вытесняют имя: обрыв каждого пятого пакета портит связь сильнее
 * лишних двадцати миллисекунд, а по одному отклику этого не видно.
 */
function Target({ probe }: { probe: Probe }) {
  const reading = probe.reading;
  const lost = reading !== null && reading.loss > 0;

  return (
    <div className="net__cell" title={probe.target}>
      <div className="net__line">
        <span
          className="net__half"
          data-accent={reading === null ? "neutral" : quality(reading.latency, LATENCY)}
        >
          <StatusDot glow />
          <BigStat size="sm" value={ms(reading?.latency)} />
        </span>

        {/* Разброс отвечает не «далеко ли», а «ровно ли»: рвутся разговор и
            игра именно от него, и своя точка ему поэтому нужна */}
        <span
          className="net__half net__half--jitter"
          data-accent={
            reading?.jitter == null ? "neutral" : quality(reading.jitter, JITTER)
          }
        >
          <StatusDot glow />
          <BigStat
            size="sm"
            value={reading?.jitter == null ? undefined : `±${ms(reading.jitter)}`}
          />
        </span>
      </div>

      <span className="net__hint tabular">
        {lost ? `loss ${Math.round(reading.loss * 100)} %` : probe.name}
      </span>
    </div>
  );
}


/** Скорость и счётчик NEW; прежний Flow выше остаётся только у Classic. */
function ReferenceFlow({ direction, speed, total, live }: {
  direction: "down" | "up";
  speed: Sensor | undefined;
  total: Sensor | undefined;
  live: boolean;
}) {
  const t = useText();
  const { value, unit } = split(speed);
  const down = direction === "down";
  return (
    <div className="net-ref__flow" data-direction={direction}>
      <div className="net-ref__flow-label">
        <span className="net-ref__arrow" data-accent={live ? (down ? "net-down" : "net-up") : "neutral"}>
          {down ? <DownArrow /> : <UpArrow />}
        </span>
        <span>{down ? "Download" : "Upload"}</span>
      </div>
      <div className="net-ref__speed tabular">
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
        <ReferenceFlow direction="down" speed={network?.download} total={network?.totalDownloaded} live={network !== undefined} />
        <ReferenceFlow direction="up" speed={network?.upload} total={network?.totalUploaded} live={network !== undefined} />
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
  useContentHeight(content, reference && probes.length > 0, [design, view, size].join("|"));

  // Адаптер выбирается по трафику: имена локализованы, виртуальных несколько
  const network = activeNetwork(telemetry);
  const live = network !== undefined;

  return (
    <div className="widget">
      <GlassCard accent="neutral" flush>
        {reference ? (
          <ReferenceNetwork network={network} probes={probes} view={view} size={size} content={content} />
        ) : (
        <div className="net">
          <div className="net__row net__row--two">
            <Flow
              accent={live ? "net-down" : "neutral"}
              arrow={<DownArrow />}
              speed={network?.download}
              total={network?.totalDownloaded}
            />
            <Flow
              accent={live ? "net-up" : "neutral"}
              arrow={<UpArrow />}
              speed={network?.upload}
              total={network?.totalUploaded}
            />
          </div>

          <div className="net__row net__row--three">
            {probes.map((probe) => (
              <Target key={probe.target} probe={probe} />
            ))}
          </div>
        </div>
        )}
      </GlassCard>
    </div>
  );
}
