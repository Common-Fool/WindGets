import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import {
  runSpeedtest,
  stageLabel,
  useSpeedtest,
  type Report,
  type Snapshot,
} from "../../lib/speedtest";
import { useWindowDrag } from "../../lib/drag";
import { useText, type Text } from "../../lib/i18n";
import { NO_VALUE } from "../../lib/format";
import { useLook } from "../../lib/look";
import { useContentHeight } from "../../lib/grow";
import { useWidgetView, useWidgetLayout, useWidgetSize } from "../../lib/view";
import { GlassCard } from "../../components/GlassCard";
import { BigStat } from "../../components/BigStat";
import { Trend } from "../../components/Trend";
import { Arc } from "../../components/Arc";
import { Button } from "../../components/Button";
import { GaugeIcon } from "../../components/Icons";
import { StatusDot } from "../../components/StatusDot";
import { DownArrow, UpArrow } from "../../components/Arrows";
import { SpeedColumns } from "./SpeedColumns";
import "./speedtest.css";
import "./speedtest-reference.css";

/**
 * Сколько кнопка недоступна после отказа.
 *
 * Отказ у Cloudflare один и тот же — «замеров было слишком много», и кнопка,
 * нажатая в этот момент ещё раз, сжигает лимит надолго. После удачного замера
 * запрета нет: он мешал бы, а лимит держится и так.
 */
const LOCKOUT = 60_000;

function report(view: Snapshot): Report {
  return view.status === "idle" ? { download: null, upload: null, latency: null } : view.report;
}

/** Скорость с одним знаком: сотые в замере всё равно не воспроизводятся. */
function speed(value: number | null): string | undefined {
  return value === null ? undefined : value.toFixed(1);
}

function reasonOf(view: Snapshot): string | undefined {
  return view.status === "failed" ? view.reason : undefined;
}

export function SpeedtestWidget() {
  const view = useSpeedtest();
  const t = useText();
  const { effectiveDesign } = useLook();
  const [failure, setFailure] = useState<string | undefined>(undefined);
  const locked = useLockout(view);
  const widgetView = useWidgetView();
  const columns = widgetView === 1 || widgetView === 3;
  const separateColumns = widgetView === 3; // два ряда с общей шкалой
  const gaugeOnly = widgetView === 2; // только кольцо замера, истории нет вовсе
  const size = useWidgetSize();
  const content = useRef<HTMLDivElement>(null);
  useWindowDrag();

  const measured = report(view);
  const running = view.status === "running";
  const reason = failure ?? reasonOf(view);

  // Отказ запуска («Уже меряем» от второго нажатия) живёт до начала замера:
  // иначе он стоял бы под итогами и после удачного конца
  useEffect(() => {
    if (view.status === "running") setFailure(undefined);
  }, [view.status]);

  const isUpload = running && view.stage === "upload";
  const activeSpeed = isUpload ? measured.upload : measured.download;
  const activeAccent = isUpload ? "net-up" : "net-down";

  const downloads = column(view.history, (entry) => entry.download);
  const uploads = column(view.history, (entry) => entry.upload);

  /*
   * Кольцо показывает идущий этап; в покое внутри стоит запуск.
   *
   * Потолок шкалы не задан числом: он берётся по наибольшему из того, что тут
   * когда-либо мерили, и не опускается ниже опорных пятисот. Постоянный
   * потолок либо прижимал бы дугу к началу на медленном канале, либо упирался
   * бы в край на быстром.
   */
  const shown = running ? activeSpeed : measured.download;
  const ceiling = Math.max(REFERENCE, ...downloads, ...uploads, shown ?? 0);
  const share = shown === null ? null : Math.min(shown / ceiling, 1);

  const start = () => {
    setFailure(undefined);
    // Ход замера приходит событием: своё состояние тут заводить нечего
    runSpeedtest().catch((cause: unknown) => setFailure(String(cause)));
  };

  // У истории своя высота, у итогов — собственная. Измерение не зависит от
  // прежней высоты окна, поэтому убранный блок не оставляет пустого места.
  useContentHeight(content, effectiveDesign === "app",
    `app|${widgetView}|${size}|${view.status}|${running ? view.stage : ""}|${reason ?? ""}|${t.language}`);

  // Состояние замера, отказ и lockout принадлежат этому компоненту:
  // смена дизайна меняет только содержимое и не запускает таймер заново.
  if (effectiveDesign === "reference") {
    return (
      <ReferenceSpeedtest view={view} t={t} measured={measured} columns={columns} separateColumns={separateColumns}
        gaugeOnly={gaugeOnly} running={running} reason={reason} locked={locked}
        isUpload={isUpload} shown={shown} share={share} downloads={downloads}
        uploads={uploads} onStart={start} />
    );
  }

  return (
    <div className="widget speedtest-app-widget">
      <GlassCard accent={activeAccent}>
        <div className="speed" data-gauge={running || gaugeOnly ? "true" : undefined} ref={content}>
          {/* История пополнится только в конце; во время замера виден спидометр. */}
          {!running && !gaugeOnly && (
            columns ? (
              <SpeedColumns history={view.history} variant={separateColumns ? "rows" : "pairs"} />
            ) : (
              <div className="speed__chart">
                <Trend
                  series={[
                    // Закрашена только загрузка: две заливки внахлёст дают
                    // серое пятно вместо графика
                    { values: downloads, accent: "net-down", label: "Download", fill: true },
                    { values: uploads, accent: "net-up", label: "Upload", dashed: true },
                  ]}
                  marks={marksOf(downloads, uploads)}
                />
              </div>
            )
          )}

          {(running || gaugeOnly) && (
            <div className="speed__dial">
              {running && <span className="speed__phase" role="status">{stageLabel(t, view.stage)}</span>}
              <SpeedGauge shown={shown} share={share} isUpload={isUpload} running={running}
                action={<SpeedAction t={t} locked={locked} onStart={start} hero />} />
            </div>
          )}

          <div className="speed__results" data-action={!running && !gaugeOnly ? "true" : undefined}>
            <div className="duo duo--three">
              <div className="duo__side" data-accent="net-down">
                <span className="duo__label" title="Download">
                  <DownArrow />
                  <span className="duo__label-text">Download</span>
                </span>
                <BigStat value={speed(measured.download)} unit="Mbps" unitPlacement="below" align="center" />
              </div>
              <div className="duo__side" data-accent="net-up">
                <span className="duo__label" title="Upload">
                  <UpArrow />
                  <span className="duo__label-text">Upload</span>
                </span>
                <BigStat value={speed(measured.upload)} unit="Mbps" unitPlacement="below" align="center" />
              </div>
              <div className="duo__side" data-accent="neutral">
                <span className="duo__label" title="Ping">
                  <StatusDot glow />
                  <span className="duo__label-text">Ping</span>
                </span>
                <BigStat value={measured.latency !== null ? measured.latency.toFixed(0) : undefined} unit="ms" unitPlacement="below" align="center" />
              </div>
            </div>
            {!running && !gaugeOnly && <SpeedAction t={t} locked={locked} onStart={start} />}
          </div>
          {reason !== undefined && <p className="speed__note speed__note--failed" role="alert">{reason}</p>}
        </div>
      </GlassCard>
    </div>
  );
}

type ReferenceProps = {
  view: Snapshot;
  t: Text;
  measured: Report;
  columns: boolean;
  separateColumns: boolean;
  gaugeOnly: boolean;
  running: boolean;
  reason: string | undefined;
  locked: number;
  isUpload: boolean;
  shown: number | null;
  share: number | null;
  downloads: number[];
  uploads: number[];
  onStart: () => void;
};

/**
 * Две утверждённые компоновки GPT. Вид истории остаётся прежней независимой
 * осью: 0 — график, 1 — пары, 2 — только замер, 3 — два ряда; layout 0/1 — A/B.
 */
function ReferenceSpeedtest({ view, t, measured, columns, separateColumns, gaugeOnly, running, reason,
  locked, isUpload, shown, share, downloads, uploads, onStart }: ReferenceProps) {
  const savedLayout = useWidgetLayout();
  const savedSize = useWidgetSize();
  const layout = !gaugeOnly && savedLayout === 1 ? "b" : "a";
  const size = gaugeOnly || savedSize === 1 ? 1 : 0;
  const content = useRef<HTMLDivElement>(null);
  const stage = view.status === "running" ? view.stage : undefined;

  // Меряем весь блок, включая кнопку, город и многострочный отказ. Ширину
  // задаёт выбранный размер, высота не зависит от нынешней высоты окна.
  useContentHeight(content, true,
    `reference|${layout}|${size}|${separateColumns ? 3 : columns ? 1 : gaugeOnly ? 2 : 0}|${view.status}|${stage ?? ""}|${reason ?? ""}|${t.language}`);

  const button = <SpeedAction t={t} locked={locked} onStart={onStart} hero={gaugeOnly} />;

  return (
    <div className="widget speedtest-reference-widget">
      <GlassCard accent={isUpload ? "net-up" : "net-down"}>
        <div className="speed-reference" data-layout={layout} data-size={size}
          data-running={running ? "true" : undefined} data-gauge={gaugeOnly ? "true" : undefined} ref={content}>
          {gaugeOnly ? (
            <>
              <div className="speed-reference__dial">
                {running && view.status === "running" && <p className="speed-reference__current-phase" role="status">
                  {stageLabel(t, view.stage)}{view.server !== null ? ` · ${view.server}` : ""}
                </p>}
                <SpeedGauge shown={shown} share={share} isUpload={isUpload} running={running} action={button} />
              </div>
              <ReferenceResults measured={measured} />
            </>
          ) : running && view.status === "running" ? (
            <div className="speed-reference__run">
              <ReferencePhases stage={view.stage} t={t} />
              <p className="speed-reference__server">
                {view.server !== null ? <><span>M-Lab</span><span aria-hidden="true">·</span><span>{view.server}</span></>
                  : stageLabel(t, view.stage)}
              </p>
              <SpeedGauge shown={shown} share={share} isUpload={isUpload} running />
              <div className="speed-reference__current-context">
                <span className="speed-reference__current-phase" role="status">{stageLabel(t, view.stage)}</span>
                {measured.latency !== null && (
                  <span className="speed-reference__ping" data-accent="neutral">
                    <StatusDot />
                    <span>Ping {measured.latency.toFixed(0)} ms</span>
                  </span>
                )}
              </div>
            </div>
          ) : (
            <>
              <ReferenceResults measured={measured} action={button} beside={layout === "b"} />
              <div className="speed-reference__history">
                {columns ? (
                  <div className="speed-reference__chart speed-reference__chart--columns">
                    <SpeedColumns history={view.history} variant={separateColumns ? "rows" : "pairs"} />
                    {!separateColumns && <ReferenceLegend />}
                  </div>
                ) : (
                  <div className="speed-reference__chart">
                    <Trend series={[
                      { values: downloads, accent: "net-down", label: "Download", fill: true },
                      { values: uploads, accent: "net-up", label: "Upload", dashed: true },
                    ]} marks={layout === "b" && size === 1 ? [] : marksOf(downloads, uploads)} />
                  </div>
                )}
              </div>
            </>
          )}
          {reason !== undefined && (
            <p className="speed-reference__message" data-failed="true" role="alert">{reason}</p>
          )}
        </div>
      </GlassCard>
    </div>
  );
}

/** В B запуск справа от значения Ping, в A — четвёртая ячейка итогов. */
function ReferenceResults({ measured, action, beside = false }: {
  measured: Report;
  action?: ReactNode;
  beside?: boolean;
}) {
  return (
    <div className="speed-reference__results" data-action={action !== undefined && !beside ? "true" : undefined}>
      <ReferenceMetric direction="down" label="Download" value={measured.download} />
      <ReferenceMetric direction="up" label="Upload" value={measured.upload} />
      <ReferenceMetric direction="ping" label="Ping" value={measured.latency} action={beside ? action : undefined} />
      {!beside && action}
    </div>
  );
}

function ReferenceMetric({ direction, label, value, action }: {
  direction: "down" | "up" | "ping";
  label: "Download" | "Upload" | "Ping";
  value: number | null;
  action?: ReactNode;
}) {
  const formatted = direction === "ping" ? value === null ? undefined : value.toFixed(0) : speed(value);
  // Пять знаков — утверждённые 566.3. Более быстрый канал помещается
  // в узкую тройку целиком, не обрезая важное число эллипсисом.
  const digits = Math.max(action === undefined ? 5 : 2, formatted?.length ?? 1);
  return (
    <div className="speed-reference__metric" data-accent={direction === "down" ? "net-down" : direction === "up" ? "net-up" : "neutral"}
      style={{ "--speed-reference-digits": digits } as CSSProperties}>
      <span className="speed-reference__metric-label" title={label}>
        {direction === "down" ? <DownArrow /> : direction === "up" ? <UpArrow /> : <StatusDot />}
        <span>{label}</span>
      </span>
      {action === undefined ? (
        <BigStat value={formatted} unit={direction === "ping" ? "ms" : "Mbps"} unitPlacement="below" />
      ) : (
        <div className="speed-reference__metric-reading">
          <div className="speed-reference__metric-value">
            <span className={`stat__value tabular${formatted === undefined ? " stat__value--empty" : ""}`}>
              {formatted ?? NO_VALUE}
            </span>
            <span className="stat__unit">{direction === "ping" ? "ms" : "Mbps"}</span>
          </div>
          {action}
        </div>
      )}
    </div>
  );
}

function SpeedAction({ t, locked, onStart, hero = false }: { t: Text; locked: number; onStart: () => void; hero?: boolean }) {
  const label = locked === 0 ? t("speedtest.measure") : t("speedtest.lockedFor", { seconds: locked });
  return (
    <div className="speed-action" data-hero={hero ? "true" : undefined}>
      <Button variant="outline" icon={hero ? (
        <svg viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true" fill="currentColor"
          stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round">
          <path d="M8 4.5 20 12 8 19.5Z" />
        </svg>
      ) : <GaugeIcon />} disabled={locked !== 0} onClick={onStart} label={label} />
      {locked !== 0 && <span className="speed__wait" title={label}>{locked}</span>}
    </div>
  );
}

/** Общий спидометр: стрелка и скорость во время замера, запуск внутри дуги в покое. */
function SpeedGauge({ shown, share, isUpload, running, action }: {
  shown: number | null;
  share: number | null;
  isUpload: boolean;
  running: boolean;
  action?: ReactNode;
}) {
  const formatted = speed(shown);
  return (
    <div className="speed-gauge" data-accent={isUpload ? "net-up" : "net-down"} data-running={running ? "true" : undefined}
      style={{ "--speed-gauge-digits": Math.max(5, formatted?.length ?? 1),
        "--speed-gauge-angle": `${-110 + (share ?? 0) * 220}deg` } as CSSProperties}>
      <Arc value={running ? share : null} span={220} thickness={3}>
        {running ? (
          <>
            {share !== null && <svg className="speed-gauge__needle" viewBox="0 0 100 100" aria-hidden="true">
              <path d="M50 15 53 50 50 53 47 50Z" />
              <circle cx="50" cy="50" r="3" />
            </svg>}
            <div className="speed-gauge__reading">
              <BigStat value={formatted} align="center" />
              <p className="speed-gauge__unit"><span>Mbps</span>{isUpload ? <UpArrow /> : <DownArrow />}</p>
            </div>
          </>
        ) : action}
      </Arc>
    </div>
  );
}

function ReferencePhases({ stage, t }: { stage: "locate" | "download" | "upload"; t: Text }) {
  const stages = ["locate", "download", "upload"] as const;
  const active = stages.indexOf(stage);
  return (
    <ol className="speed-reference__phases" aria-label={t("speedtest.phases")}>
      {stages.map((phase, index) => (
        <li key={phase} className="speed-reference__phase"
          data-phase={index === active ? "active" : index < active ? "done" : "waiting"}
          aria-current={index === active ? "step" : undefined}>
          <span className="speed-reference__phase-mark" aria-hidden="true" />
          <span>{phase === "locate" ? t("speedtest.phase.server") : phase === "download" ? "Download" : "Upload"}</span>
        </li>
      ))}
    </ol>
  );
}

/** Столбики своей легенды не имеют; её значения и штрихи те же, что у Trend. */
function ReferenceLegend() {
  return (
    <div className="speed-reference__legend">
      <span className="speed-reference__key" data-accent="net-down"><span className="speed-reference__sample" aria-hidden="true" />Download</span>
      <span className="speed-reference__key" data-accent="net-up"><span className="speed-reference__sample speed-reference__sample--dashed" aria-hidden="true" />Upload</span>
    </div>
  );
}

/** Ряд значений из истории, без незамеренных. */
function column(history: Report[], pick: (entry: Report) => number | null): number[] {
  return history
    .map(pick)
    .filter((value): value is number => value !== null && Number.isFinite(value));
}

/**
 * Опорная отметка на шкале.
 *
 * Ровные сотни видны сеткой и подписи не требуют, а вот пятьсот — тот
 * уровень, относительно которого канал и оценивают.
 */
const REFERENCE = 500;

/**
 * Что подписать на шкале: последнее и пиковое по каждому ряду плюс опорное.
 *
 * Ровные деления не подписываем вовсе: знать хочется «сколько сейчас» и
 * «сколько бывало», а не то, где проходит очередная сотня.
 */
function marksOf(downloads: number[], uploads: number[]): number[] {
  const marks = [REFERENCE];

  for (const values of [downloads, uploads]) {
    if (values.length === 0) continue;
    marks.push(values[values.length - 1]!, Math.max(...values));
  }

  return marks;
}

/**
 * Сколько секунд кнопка ещё недоступна.
 *
 * Считаем по переходу в отказ, а не по самому состоянию: оно живёт в Rust и
 * переживает закрытие окна, иначе открытое заново окно запиралось бы на минуту
 * заново из-за отказа, случившегося вчера.
 */
function useLockout(view: Snapshot): number {
  const [until, setUntil] = useState(0);
  const [left, setLeft] = useState(0);
  const previous = useRef(view.status);

  useEffect(() => {
    // Только из идущего замера: открытое окно сперва видит «простой» до
    // ответа Rust, и вчерашний отказ выглядел бы новым переходом
    if (view.status === "failed" && previous.current === "running") {
      setUntil(Date.now() + LOCKOUT);
    }
    previous.current = view.status;
  }, [view.status]);

  useEffect(() => {
    if (until === 0) return;

    const tick = () => {
      const remaining = Math.ceil((until - Date.now()) / 1000);
      setLeft(Math.max(remaining, 0));
      if (remaining <= 0) setUntil(0);
    };

    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [until]);

  return until === 0 ? 0 : left;
}
