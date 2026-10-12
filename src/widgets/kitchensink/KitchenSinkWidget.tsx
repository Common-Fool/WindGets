import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTelemetry, type SourceState } from "../../lib/telemetry";
import { useSeries } from "../../lib/history";
import { shareOf, showPair, showReading } from "../../lib/format";
import { spaceAccent, type Accent } from "../../lib/accent";
import {
  capacityOf,
  cpuLoad,
  disks,
  gpuClock,
  gpuFans,
  gpuLoad,
  gpuMemoryClock,
  gpuPower,
  gpuSecondTemp,
  gpuTemp,
  gpuVramLoad,
  gpuVramTotal,
  gpuVramUsed,
  ramLoad,
  ramUsed,
} from "../../lib/sensors";
import { GlassCard } from "../../components/GlassCard";
import { MetricPair } from "../../components/MetricPair";
import { BigStat } from "../../components/BigStat";
import { Columns } from "../../components/Columns";
import { Trend } from "../../components/Trend";
import { Bar } from "../../components/Bar";
import { Arc } from "../../components/Arc";
import { StatusDot } from "../../components/StatusDot";
import { SegmentMeter } from "../../components/SegmentMeter";
import { Slider } from "../../components/Slider";
import { Button } from "../../components/Button";
import { Toggle } from "../../components/Toggle";
import { Check } from "../../components/Check";
import "../hardware/sections.css";
import "./kitchensink.css";

const ACCENTS: Accent[] = [
  "cpu",
  "gpu",
  "ram",
  "storage",
  "net-down",
  "net-up",
  "neutral",
  "ok",
  "warn",
  "alert",
  "danger",
];

/**
 * Занятость накопителя по одной на каждую ступень цвета.
 *
 * Свободно 30, 15, 7 и 3 процента: зелёная, жёлтая, оранжевая, красная.
 * Живые накопители все ступени показать не могут — для этого их пришлось бы
 * забить.
 */
const SPACE_STEPS = [70, 85, 93, 97];

function Section({ legend, children }: { legend: string; children: React.ReactNode }) {
  return (
    <section className="sink__section">
      <p className="sink__legend caps">{legend}</p>
      {children}
    </section>
  );
}

export function KitchenSinkWidget() {
  const state = useTelemetry();

  const [length, setLength] = useState(16);
  const [upper, setUpper] = useState(true);
  const [digits, setDigits] = useState(true);
  const [symbols, setSymbols] = useState(false);

  return (
    <div className="sink">
      <div className="sink__bar" data-tauri-drag-region="">
        <span className="sink__title caps">Витрина компонентов</span>
        <span className="sink__hint">окно тянется за эту полосу</span>
        <button className="sink__close" onClick={() => void getCurrentWindow().close()}>
          ×
        </button>
      </div>

      <div className="sink__scroll">
        <Section legend="Палитра — цвет закреплён за метрикой">
          <div className="sink__swatches">
            {ACCENTS.map((accent) => (
              <span key={accent} className="sink__swatch" data-accent={accent}>
                <span className="sink__chip" />
                {accent}
              </span>
            ))}
          </div>
        </Section>

        <Section legend="Живые данные — карточка, график, ряды">
          <div className="sink__grid">
            <GpuCard state={state} />
            <CpuRamCard state={state} />
            <StorageCard state={state} />
            <ColumnsCard state={state} />
            <TrendCard state={state} />
          </div>
        </Section>

        <Section legend="Крупная цифра — единица под числом и рядом с ним">
          <div className="sink__grid">
            <GlassCard title="Сеть" centered accent="net-down">
              <div className="sink__pair">
                <div data-accent="net-down">
                  <span className="sink__label">
                    <StatusDot glow />
                    DOWN
                  </span>
                  <BigStat
                    value="49.0"
                    unit="КБ/с"
                    unitPlacement="below"
                    caption="Всего: 3,7 ГБ"
                    align="center"
                  />
                </div>
                <div className="sink__divider" data-accent="net-up">
                  <span className="sink__label">
                    <StatusDot glow />
                    UP
                  </span>
                  <BigStat
                    value="11.3"
                    unit="КБ/с"
                    unitPlacement="below"
                    caption="Всего: 1,8 ГБ"
                    align="center"
                  />
                </div>
              </div>
            </GlassCard>

            <GlassCard title="Размеры" accent="ram">
              <div className="sink__stack">
                <BigStat value="452.8" unit="Мбит/с" size="lg" caption="крупный размер" />
                <BigStat value="57" unit="%" caption="обычный размер" />
              </div>
            </GlassCard>
          </div>
        </Section>

        <Section legend="Индикаторы — кольцо и дуга это один компонент">
          <div className="sink__grid">
            <GlassCard title="Пинг" centered accent="ok">
              <div className="sink__center">
                <Arc value={null} span={360} size={104} thickness={6}>
                  <span className="sink__strong">Online</span>
                  <BigStat value="20" unit="мс" align="center" />
                  <span className="sink__caption">Отлично</span>
                </Arc>
              </div>
            </GlassCard>

            <GlassCard title="Спидтест" centered accent="ram">
              <div className="sink__center">
                <Arc value={0.72} span={220} size={168} thickness={10}>
                  <BigStat
                    value="452.8"
                    unit="Мбит/с"
                    unitPlacement="below"
                    size="lg"
                    align="center"
                  />
                </Arc>
              </div>
              <div className="sink__pair" style={{ marginTop: "var(--space-4)" }}>
                <BigStat value="452.8" caption="ЗАГРУЗКА" align="center" />
                <BigStat value="178.1" caption="ОТДАЧА" align="center" />
              </div>
            </GlassCard>

            <GlassCard title="Заполнение дуги" accent="storage">
              <div className="sink__inline">
                <Arc value={0.25} span={220} size={72} thickness={7} />
                <Arc value={0.6} span={360} size={72} thickness={7} />
                <Arc value={1} span={300} size={72} thickness={7} />
              </div>
            </GlassCard>
          </div>
        </Section>

        <Section legend="Интерактив — генератор паролей и кнопки">
          <div className="sink__grid">
            <GlassCard title="Генератор паролей" accent="neutral">
              <div className="sink__stack">
                <div className="sink__password">az9U(*w6S&lt;f6gSa:</div>
                <SegmentMeter level={3} total={4} accent="ok" />
                <div className="sink__inline">
                  <span className="sink__caption">Длина</span>
                  <Slider value={length} min={6} max={48} onChange={setLength} />
                  <span className="sink__strong tabular">{length}</span>
                </div>
                <div className="sink__inline">
                  <Toggle label="A-Z" active={upper} onChange={setUpper} />
                  <Toggle label="0-9" active={digits} onChange={setDigits} />
                  <Toggle label="!@#" active={symbols} onChange={setSymbols} />
                </div>
                <div className="sink__inline">
                  <Button icon="⟳">Сгенерировать</Button>
                  <Button variant="outline" icon="⟳">
                    Повторить замер
                  </Button>
                  <Button busy icon="⟳">
                    Идёт замер
                  </Button>
                </div>
              </div>
            </GlassCard>

            <GlassCard title="Стойкость" accent="neutral">
              <div className="sink__stack">
                <SegmentMeter level={1} total={4} accent="danger" />
                <SegmentMeter level={2} total={4} accent="warn" />
                <SegmentMeter level={4} total={4} accent="ok" />
              </div>
            </GlassCard>

            {/* Галочка живёт в таблице настроек, где подпись стоит в шапке
                столбца, — отсюда и три состояния подряд без подписей */}
            <GlassCard title="Галочки" accent="gpu">
              <div className="sink__inline">
                <Check checked label="Включено" onChange={() => {}} />
                <Check checked={false} label="Выключено" onChange={() => {}} />
                <Check checked disabled label="Применять не к чему" onChange={() => {}} />
                <Check checked={false} disabled label="Недоступно" onChange={() => {}} />
              </div>
            </GlassCard>
          </div>
        </Section>

        <Section legend="Нет данных — везде прочерк, нигде не ноль">
          <div className="sink__grid">
            <GlassCard title="Пустые состояния" value="—" accent="storage">
              <div className="sink__stack">
                <Columns values={[]} />
                <Bar fill={null} />
                <Bar fill={null} thickness="thin" />
                <BigStat unit="%" caption="датчик молчит" />
              </div>
              <div className="pairs">
                <MetricPair label="Значения нет" solo />
                <MetricPair label="Пустая строка" first="" solo />
                <MetricPair label="Есть значение" first="42,0 °C" firstHint="для сравнения" solo />
              </div>
            </GlassCard>

            <GlassCard title="Дуга без значения" centered accent="neutral">
              <div className="sink__inline">
                <Arc value={null} span={220} size={80} thickness={8} />
                <Arc value={null} span={360} size={80} thickness={8} />
              </div>
            </GlassCard>
          </div>
        </Section>
      </div>
    </div>
  );
}

/* Датчики — по ролям, как у настоящих карточек: витрина открывается и на
   чужом железе. Ряды — строки пары, теми же `MetricPair`, что у карточек */
function GpuCard({ state }: { state: SourceState }) {
  const load = gpuLoad(state);
  const series = useSeries(load?.path);
  const fans = gpuFans(state);
  const name = state.status === "ok" ? state.snapshot.roles.gpu?.name : undefined;

  return (
    <GlassCard title={name ?? "GPU"} value={showReading(load)} accent="gpu">
      <Columns values={series} height={52} />
      <div className="pairs">
        <MetricPair
          label="VRAM"
          first={showPair(gpuVramUsed(state), gpuVramTotal(state))}
          firstHint={showReading(gpuVramLoad(state))}
          solo
        />
        <MetricPair
          label="Temperature"
          first={showReading(gpuTemp(state))}
          second={showReading(gpuSecondTemp(state))}
          firstHint="core"
          secondHint="memory"
        />
        <MetricPair label="Power" first={showReading(gpuPower(state))} solo />
        <MetricPair label="Fan" first={showReading(fans[0])} second={showReading(fans[1])} />
        <MetricPair
          label="Clock"
          first={showReading(gpuClock(state))}
          second={showReading(gpuMemoryClock(state))}
        />
      </div>
    </GlassCard>
  );
}

function CpuRamCard({ state }: { state: SourceState }) {
  const cpu = cpuLoad(state);
  const ram = ramLoad(state);
  const cpuSeries = useSeries(cpu?.path);

  return (
    <GlassCard title="Процессор и память" centered accent="cpu">
      <Columns values={cpuSeries} height={40} />
      <div className="sink__pair" style={{ marginTop: "var(--space-4)" }}>
        <div className="sink__stack" data-accent="cpu">
          <BigStat value={cpu?.value?.toFixed(0)} unit="%" caption="CPU" align="center" />
          <Bar fill={shareOf(cpu)} thickness="thin" />
        </div>
        <div className="sink__stack sink__divider" data-accent="ram">
          <BigStat
            value={ram?.value?.toFixed(0)}
            unit="%"
            caption={showReading(ramUsed(state))}
            align="center"
          />
          <Bar fill={shareOf(ram)} thickness="thin" />
        </div>
      </div>
    </GlassCard>
  );
}

/* Строка показателя: кольцо с цифрой внутри слева, история столбиками справа.
   Число столбиков диаграмма считает сама по доставшейся ширине */
function ColumnsCard({ state }: { state: SourceState }) {
  const core = gpuLoad(state);
  const series = useSeries(core?.path);

  return (
    <GlassCard title="Кольцо и столбики" accent="gpu">
      <div className="sink__stack">
        <div className="gauge">
          <div className="gauge__dial">
            <Arc value={shareOf(core)} size={104}>
              <BigStat
                value={core?.value?.toFixed(0)}
                unit="%"
                align="center"
                caption={<span className="caps">Core</span>}
              />
            </Arc>
          </div>
          <Columns values={series} height={104} />
        </div>
        <span className="sink__caption">Столбиков столько, сколько влезло по ширине</span>
        <Columns values={[]} />
        <span className="sink__caption">Пустая история: одни треки, расти нечему</span>
      </div>
    </GlassCard>
  );
}

/* Тот же график, что у ломаной выше, но на несколько рядов и с легендой:
   заливки полупрозрачные, поэтому нижний ряд виден из-под верхнего */
function TrendCard({ state }: { state: SourceState }) {
  const core = useSeries(gpuLoad(state)?.path);
  const memory = useSeries(gpuVramLoad(state)?.path);

  return (
    <GlassCard title="График на два ряда" accent="gpu">
      <div className="sink__stack">
        <div className="sink__chart">
          <Trend
            series={[
              { values: core, accent: "net-down", label: "Ядро", fill: true },
              { values: memory, accent: "net-up", label: "Память" },
            ]}
            marks={[core[core.length - 1] ?? 0, memory[memory.length - 1] ?? 0, 50]}
            step={25}
            formatMark={(value) => `${value.toFixed(0)} %`}
          />
        </div>
        <span className="sink__caption">
          Сетка ровным шагом, подписаны только последние значения и опорное
        </span>
        <div className="sink__chart">
          <Trend series={[{ values: [], accent: "gpu", label: "Пусто" }]} step={25} />
        </div>
        <span className="sink__caption">Пустой ряд: рисовать нечего, места нет</span>
      </div>
    </GlassCard>
  );
}

function StorageCard({ state }: { state: SourceState }) {
  return (
    <GlassCard title="Storage" centered accent="storage">
      <div className="sink__stack">
        {disks(state).map((disk) => (
          <div key={disk.name} className="sink__stack">
            {/* Скорости у некоторых NVMe LHM не отдаёт — тут и видно прочерк */}
            <MetricPair
              label={capacityOf(disk.name) ?? disk.name}
              first={showPair(disk.free, disk.total)}
              second={showPair(disk.readRate, disk.writeRate)}
              firstHint={showReading(disk.usedShare)}
              secondHint="R / W"
            />
            <Bar fill={shareOf(disk.usedShare)} />
          </div>
        ))}

        <MetricPair label="Цвет по остатку" first="30 · 15 · 7 · 3 %" solo />
        {SPACE_STEPS.map((used) => (
          <div key={used} data-accent={spaceAccent(used)}>
            <Bar fill={used / 100} />
          </div>
        ))}
      </div>
    </GlassCard>
  );
}
