import { useRef } from "react";
import { useLook } from "../../lib/look";
import { HardwareLoads } from "./HardwareLoads";
import { useTelemetry, type SourceState } from "../../lib/telemetry";
import { needsDriver, usePawnio } from "../../lib/pawnio";
import { useSettings, type Metric } from "../../lib/settings";
import { useContentHeight } from "../../lib/grow";
import { useSeries } from "../../lib/history";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import {
  boardTemp,
  chassisFan,
  cpuClock,
  cpuFan,
  cpuLoad,
  cpuPower,
  cpuTemp,
  cpuVoltage,
  ramLoad,
  ramTotal,
  ramUsed,
} from "../../lib/sensors";
import { NO_VALUE, gigabytes, shareOf, showLoad, usedOfTotal } from "../../lib/format";
import { GlassCard } from "../../components/GlassCard";
import { MetricLine } from "./MetricLine";
import { DriverGate } from "./DriverGate";
import { BigStat } from "../../components/BigStat";
import { Arc } from "../../components/Arc";
import { Columns } from "../../components/Columns";
import { Bar } from "../../components/Bar";
import { MetricLabel } from "../../components/MetricLabel";
import { CpuIcon, MemoryIcon, TemperatureIcon, FanIcon, ClockIcon, PowerIcon } from "../../components/Icons";
import "./sections.css";

/** Карточка без драйвера: снимка будто нет, и все ряды стоят прочерками. */
const WITHOUT_DRIVER: SourceState = { status: "connecting" };

/**
 * Процессор вместе с памятью.
 *
 * Парный виджет загрузки переехал сюда целиком: по строке на показатель —
 * кольцо с процентом внутри слева, история столбиками справа (вид 0) либо
 * полоса загрузки с историей под ней (вид 1). Отдельной строки «загрузка»
 * больше нет, она дублировала бы цифру в кольце. Память тут гостья, поэтому
 * у неё только загрузка: температур и частот у неё в выхлопе нет вовсе.
 *
 * Подписей под кольцами нет: число потоков на своей машине известно и так, а
 * занятые гигабайты дублировали процент, который стоит внутри кольца. Заодно
 * кольцо стало вровень с историей рядом — прежде столбик «кольцо плюс подпись»
 * был выше графика ровно на подпись.
 */
export function CpuSection() {
  const telemetry = useTelemetry();
  const driver = usePawnio();
  // Без драйвера PawnIO температура, частота и потребление приходят нулями, а
  // датчиков платы нет вовсе — карточка показала бы ледяной процессор. Поэтому
  // она заперта целиком: прочерки во весь рост и слой с предложением драйвера
  const blocked = needsDriver(driver);
  const state = blocked ? WITHOUT_DRIVER : telemetry;
  const view = useWidgetView();
  const size = useWidgetSize();
  const design = useLook().effectiveDesign;

  const load = cpuLoad(state);
  const ram = ramLoad(state);

  const loadSeries = useSeries(load?.path);
  const ramSeries = useSeries(ram?.path);

  // Снимок пришёл: отсутствие роли теперь значит «на этом железе такого нет»
  const live = state.status === "ok";

  // Ряды, снятые в настройках виджета (этап 58). Загрузки не снимаются
  const hidden = useSettings().hiddenMetrics.cpu ?? [];
  const shows = (metric: Metric) => !hidden.includes(metric);

  // Окно — высотой ровно по содержимому, как у видеокарты рядом
  const content = useRef<HTMLDivElement>(null);
  useContentHeight(content, true, `${design}|${view}|${size}|${hidden.join()}|${blocked}`, true);

  return (
    <GlassCard accent="cpu">
      {/* Обёртка — то, что меряет подгонка высоты: низ последнего ряда */}
      <div className={blocked ? "hardware-body hardware-body--blocked" : "hardware-body"} data-size={size} ref={content}>
        {design === "reference" ? (
          <HardwareLoads view={view}
            load={{ label: "CPU", value: load?.value, fill: shareOf(load), history: loadSeries, accent: "cpu" }}
            memory={{ label: "RAM", used: ramUsed(state)?.value, total: ramTotal(state), fill: shareOf(ram), history: ramSeries }} />
        ) : view === 1 ? (
          <div className="bars">
            <div className="bar-row" data-accent="cpu">
              <div className="bar-row__head">
                <span className="bar-row__label caps"><MetricLabel icon={<CpuIcon />} badge>CPU</MetricLabel></span>
                <span className="bar-row__value tabular">{showLoad(load) ?? NO_VALUE}</span>
              </div>
              <Bar fill={shareOf(load)} />
              <Columns values={loadSeries} />
            </div>

            <div className="bar-row" data-accent="ram">
              <div className="bar-row__head">
                <span className="bar-row__label caps"><MetricLabel icon={<MemoryIcon />} compact="RAM" badge>Memory</MetricLabel></span>
                {/* Гигабайты вместо процента: процент рисует полоса под строкой */}
                <span className="bar-row__value tabular">
                  {usedOfTotal(ramUsed(state)?.value, ramTotal(state)) ?? NO_VALUE}
                  <span className="bar-row__unit">GB</span>
                </span>
              </div>
              <Bar fill={shareOf(ram)} />
              <Columns values={ramSeries} />
            </div>
          </div>
        ) : (
          <div className="gauges">
            <div className="gauge" data-accent="cpu">
              <div className="gauge__dial">
                <Arc value={shareOf(load)}>
                  <BigStat
                    value={load?.value?.toFixed(0)}
                    unit="%"
                    align="center"
                    caption={<span className="caps"><MetricLabel icon={<CpuIcon />}>CPU</MetricLabel></span>}
                  />
                </Arc>
              </div>
              <Columns values={loadSeries} />
            </div>

            {/* Гигабайты, а не процент, — как у видеокарты рядом и как в «полосах» */}
            <div className="gauge" data-accent="ram">
              <div className="gauge__dial">
                <Arc value={shareOf(ram)}>
                  <BigStat
                    value={gigabytes(ramUsed(state)?.value)}
                    unit="GB"
                    align="center"
                    caption={<span className="caps"><MetricLabel icon={<MemoryIcon />}>Memory</MetricLabel></span>}
                  />
                </Arc>
              </div>
              <Columns values={ramSeries} />
            </div>
          </div>
        )}

        <div className="pairs">
          {/*
            Ряды из того, что есть на этом железе (этап 57). Вентиляторы и
            температура платы зовутся так, как их назвала таблица LHM под
            известную плату; у незнакомой они «Fan #1…», угадывать там нельзя, и
            такой ячейки просто нет.
          */}
          {shows("temperature") && (
            <MetricLine
              live={live}
              label={design === "reference" ? "Temp" : "Temperature"}
              icon={<TemperatureIcon />}
              cells={[
                { reading: cpuTemp(state), hint: "CPU" },
                { reading: boardTemp(state), hint: "board" },
              ]}
            />
          )}
          {shows("fan") && (
            <MetricLine
              live={live}
              label={design === "reference" ? "Fans" : "Fan"}
              icon={<FanIcon />}
              cells={[
                { reading: cpuFan(state), hint: "CPU" },
                { reading: chassisFan(state), hint: "chassis" },
              ]}
            />
          )}
          {/*
            * Частота одиночная, но строкой пары: у видеокарты рядом все строки
            * двузначные, и одиночное значение обязано стоять в том же левом
            * столбце, что первое у соседних строк. Второй величины взять
            * неоткуда: у процессора есть только
            * `Cores (Average Effective)`, признанная ненужной.
            *
            * У потребления второй величиной с этапа 57 стоит напряжение ядра —
            * как у видеокарты. У Zen 4 своего напряжения в выхлопе нет, оно
            * берётся у чипа платы («Vcore»); нет и там — строка одиночная.
            */}
          {shows("clock") && (
            <MetricLine live={live} label="Clock" icon={<ClockIcon />} cells={[{ reading: cpuClock(state), hint: "cores" }]} />
          )}
          {/* Ватты и вольты снимаются порознь: одна строка, две галочки */}
          <MetricLine
            live={live}
            label="Power"
            icon={<PowerIcon />}
            cells={[
              ...(shows("power") ? [{ reading: cpuPower(state), hint: "package" }] : []),
              ...(shows("voltage") ? [{ reading: cpuVoltage(state), hint: "core" }] : []),
            ]}
          />
        </div>
      </div>
      {blocked && <DriverGate driver={driver} />}
    </GlassCard>
  );
}
