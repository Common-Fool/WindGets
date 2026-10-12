import { useRef } from "react";
import { useLook } from "../../lib/look";
import { HardwareLoads } from "./HardwareLoads";
import { useTelemetry } from "../../lib/telemetry";
import { useSettings, type Metric } from "../../lib/settings";
import { useContentHeight } from "../../lib/grow";
import { useSeries } from "../../lib/history";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import {
  gpuClock,
  gpuDoublesMemoryClock,
  gpuFans,
  gpuLoad,
  gpuMemoryClock,
  gpuPower,
  gpuPresent,
  gpuSecondTemp,
  gpuSecondTempKind,
  gpuTemp,
  gpuVoltage,
  gpuVramLoad,
  gpuVramTotal,
  gpuVramUsed,
} from "../../lib/sensors";
import {
  gigabytes,
  NO_VALUE,
  shareOf,
  showLoad,
  showScaled,
  usedOfTotal,
} from "../../lib/format";
import { useText } from "../../lib/i18n";
import { GlassCard } from "../../components/GlassCard";
import { MetricLine } from "./MetricLine";
import { BigStat } from "../../components/BigStat";
import { Arc } from "../../components/Arc";
import { Columns } from "../../components/Columns";
import { Bar } from "../../components/Bar";
import { MetricLabel } from "../../components/MetricLabel";
import { GpuIcon, MemoryIcon, TemperatureIcon, FanIcon, ClockIcon, PowerIcon } from "../../components/Icons";
import "./sections.css";

/**
 * Доля занятой видеопамяти: готовый процент, а без него — занятое из всего.
 * У видеокарт, где LHM процента не отдаёт, кольцо иначе стояло бы пустым.
 */
function memoryShare(load: ReturnType<typeof gpuVramLoad>, used?: number | null, total?: number | null): number | null {
  const share = shareOf(load);
  if (share !== null) return share;
  return used != null && total != null && total > 0 ? Math.min(1, Math.max(0, used / total)) : null;
}

export function GpuSection() {
  const state = useTelemetry();
  const view = useWidgetView();
  const size = useWidgetSize();
  const design = useLook().effectiveDesign;
  const t = useText();

  const load = gpuLoad(state);
  const memoryLoad = gpuVramLoad(state);
  const loadSeries = useSeries(load?.path);
  const memorySeries = useSeries(memoryLoad?.path);

  // Гигабайты для кольца памяти: датчик отдаёт байты
  const usedBytes = gpuVramUsed(state)?.value;
  const totalBytes = gpuVramTotal(state)?.value;
  const used = gigabytes(usedBytes);
  const memoryFill = memoryShare(memoryLoad, usedBytes, totalBytes);

  // Снимок пришёл: отсутствие роли теперь значит «на этом железе такого нет»
  const live = state.status === "ok";
  const present = gpuPresent(state);

  // Ряды, снятые в настройках виджета (этап 58). Загрузки не снимаются
  const hidden = useSettings().hiddenMetrics.gpu ?? [];
  const shows = (metric: Metric) => !hidden.includes(metric);

  /*
   * Окно — высотой ровно по содержимому, как у накопителей: снятый ряд или
   * датчик, которого на этом железе нет, иначе оставляли бы дыру внизу.
   * При ожидании/отказе также измеряем placeholders, чтобы после строки
   * «нет GPU» полная карточка могла восстановить свою высоту.
   */
  const content = useRef<HTMLDivElement>(null);
  useContentHeight(content, true, `${design}|${view}|${size}|${hidden.join()}|${present}`, true);

  /*
   * Дискретной видеокарты нет — только встройка или ничего (этап 57).
   * Показывать нечего, и карточка говорит это строкой, а не кольцами с
   * прочерками. Модуль при этом не выключается сам: после обновления
   * драйвера NVIDIA видеокарта бывает не видна до перезапуска, и выключенный
   * виджет пришлось бы включать заново. Видели её хоть раз за запуск — эта
   * строка уже не появится (роли помнит Rust).
   */
  if (present === false) {
    return (
      <GlassCard accent="gpu">
        <div className="hardware-body" data-size={size} ref={content}><p className="section-note">{t("hardware.noGpu")}</p></div>
      </GlassCard>
    );
  }

  const fans = gpuFans(state);
  const memoryClock = gpuMemoryClock(state);
  const secondHeat = gpuSecondTempKind(state);

  return (
    <GlassCard accent="gpu">
      {/* Обёртка — то, что меряет подгонка высоты: низ последнего ряда */}
      <div className="hardware-body" data-size={size} ref={content}>
        {/*
          Два вида верхнего блока: кольца со столбиками истории справа (вид 0)
          или полосы загрузки с историей под ними (вид 1).
        */}
        {design === "reference" ? (
          <HardwareLoads view={view}
            load={{ label: "GPU", value: load?.value, fill: shareOf(load), history: loadSeries, accent: "gpu" }}
            memory={{ label: "VRAM", used: usedBytes, total: totalBytes, fill: memoryFill, history: memorySeries }} />
        ) : view === 1 ? (
          <div className="bars">
            <div className="bar-row" data-accent="gpu">
              <div className="bar-row__head">
                <span className="bar-row__label caps"><MetricLabel icon={<GpuIcon />} badge>Core</MetricLabel></span>
                <span className="bar-row__value tabular">{showLoad(load) ?? NO_VALUE}</span>
              </div>
              <Bar fill={shareOf(load)} />
              <Columns values={loadSeries} />
            </div>

            {/*
              У памяти вместо процента занятые гигабайты из всех: процент тут же
              рисует полоса под строкой, а вопрос к видеопамяти всегда один —
              влезет ли модель. Отдельного блока с теми же гигабайтами в этом
              виде поэтому нет, он был вторым показом одного и того же.
            */}
            <div className="bar-row" data-accent="ram">
              <div className="bar-row__head">
                <span className="bar-row__label caps"><MetricLabel icon={<MemoryIcon />} compact="VRAM" badge>Memory</MetricLabel></span>
                <span className="bar-row__value tabular">
                  {usedOfTotal(gpuVramUsed(state)?.value, gpuVramTotal(state)?.value) ?? NO_VALUE}
                  <span className="bar-row__unit">GB</span>
                </span>
              </div>
              <Bar fill={memoryFill} />
              <Columns values={memorySeries} />
            </div>
          </div>
        ) : (
          <div className="gauges">
            <div className="gauge" data-accent="gpu">
              <div className="gauge__dial">
                <Arc value={shareOf(load)}>
                  <BigStat
                    value={load?.value?.toFixed(0)}
                    unit="%"
                    align="center"
                    caption={<span className="caps"><MetricLabel icon={<GpuIcon />}>Core</MetricLabel></span>}
                  />
                </Arc>
              </div>
              <Columns values={loadSeries} />
            </div>

            {/*
              В кольце памяти стоят гигабайты, а не процент, — то же правило, что
              в «полосах»: долю рисует само кольцо, а вопрос к видеопамяти всегда
              один, влезет ли модель. «Из скольких» кольцо показывает заливкой,
              поэтому число одно и внутрь помещается.
            */}
            <div className="gauge" data-accent="ram">
              <div className="gauge__dial">
                <Arc value={memoryFill}>
                  <BigStat
                    value={used}
                    unit="GB"
                    align="center"
                    caption={<span className="caps"><MetricLabel icon={<MemoryIcon />}>Memory</MetricLabel></span>}
                  />
                </Arc>
              </div>
              <Columns values={memorySeries} />
            </div>
          </div>
        )}

        {/*
          Отдельного блока видеопамяти тут больше нет ни в одном виде: он был
          вторым показом того же самого. В «полосах» гигабайты стоят в строке
          памяти, в «кольцах» — внутри кольца, а долю в обоих случаях рисует
          полоса или заливка кольца рядом.

          Убрано это заодно с выравниванием пары: у процессора такого блока нет и
          взяться ему неоткуда, поэтому в «кольцах» на его месте у соседа зияла
          дыра в полсотни точек.
        */}
        <div className="pairs">
          {/*
            Ряды собираются из того, что есть на этом железе (этап 57): ячейка
            без датчика уходит, ряд без обеих — тоже. Hotspot у RTX 50 закрыт,
            поэтому второй температурой идёт память, а у карт без неё — hot spot.
          */}
          {shows("temperature") && (
            <MetricLine
              live={live}
              label={design === "reference" ? "Temp" : "Temperature"}
              icon={<TemperatureIcon />}
              cells={[
                { reading: gpuTemp(state), hint: "core" },
                { reading: gpuSecondTemp(state), hint: secondHeat === "hotspot" ? "hot spot" : "memory" },
              ]}
            />
          )}
          {/* Кулер один — одна ячейка: прежние «GPU Fan 1» и «GPU Fan 2» его не
              видели и писали «— / —» */}
          {shows("fan") && (
            <MetricLine
              live={live}
              label={design === "reference" ? "Fans" : "Fan"}
              icon={<FanIcon />}
              cells={
                live
                  ? fans.map((fan, index) => ({ reading: fan, hint: fans.length === 1 ? "cooler" : index === 0 ? "first" : "second" }))
                  : [{ reading: undefined, hint: "first" }, { reading: undefined, hint: "second" }]
              }
            />
          )}
          {shows("clock") && (
            <MetricLine
              live={live}
              label="Clock"
              icon={<ClockIcon />}
              cells={[
                { reading: gpuClock(state), hint: "core" },
                {
                  reading: memoryClock,
                  // LHM отдаёт физическую частоту памяти NVIDIA, привычная по
                  // спецификациям вдвое выше
                  text: gpuDoublesMemoryClock(state) ? showScaled(memoryClock, 2) : undefined,
                  hint: "memory",
                },
              ]}
            />
          )}
          {/*
            * Подсказки тут не украшение: без них строка на одну ниже соседних, и
            * блок рядов расходится по высоте с карточкой процессора, стоящей
            * рядом. Заодно они и по делу — ватты у всей платы, вольты у ядра.
            */}
          {/* Ватты и вольты снимаются порознь: одна строка, две галочки */}
          <MetricLine
            live={live}
            label="Power"
            icon={<PowerIcon />}
            cells={[
              ...(shows("power") ? [{ reading: gpuPower(state), hint: "package" }] : []),
              ...(shows("voltage") ? [{ reading: gpuVoltage(state), hint: "core" }] : []),
            ]}
          />
        </div>
      </div>
    </GlassCard>
  );
}
