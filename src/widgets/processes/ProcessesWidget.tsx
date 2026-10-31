import { useRef, useState, type RefObject } from "react";
import { useText } from "../../lib/i18n";
import { formatReading } from "../../lib/format";
import { useLook, useScale } from "../../lib/look";
import { useProcesses, type Group } from "../../lib/processes";
import { useHeight } from "../../lib/size";
import { GlassCard } from "../../components/GlassCard";
import { Bar } from "../../components/Bar";
import { Toggle } from "../../components/Toggle";
import "./processes.css";
import "./processes-design.css";

/**
 * Зазор между строками в точках при кегле 100 %.
 *
 * Повторяет `--space-2` у `.proc__list`: высоту строки мы меряем, а зазор
 * между ними замерить нечем, и второе число тут дешевле похода в
 * `getComputedStyle` на каждое изменение размера. То же, что `SLOT` у
 * диаграммы столбиками.
 */
const ROW_GAP = 6;

/** По чему собран список: по процессору или по памяти. */
type Mode = "cpu" | "ram";

function amount(group: Group, mode: Mode): number {
  return mode === "cpu" ? group.cpu : group.memory;
}

function showCpu(group: Group): string {
  return `${group.cpu.toFixed(1)} %`;
}

function showMemory(group: Group): string {
  return formatReading(group.memory, "byte");
}

function Row({
  group,
  mode,
  peak,
  measured,
}: {
  group: Group;
  mode: Mode;
  peak: number;
  measured?: RefObject<HTMLLIElement | null>;
}) {
  return (
    <li className="proc__item" ref={measured}>
      <div className="proc__head">
        <span className="proc__name">
          {group.name}
          {/* Счётчик только у настоящих групп: «×1» у каждой второй строки
              был бы шумом, а место в строке дорого */}
          {group.count > 1 && <span className="proc__count"> ×{group.count}</span>}
        </span>
        <span className="proc__cpu tabular">{showCpu(group)}</span>
        <span className="proc__mem tabular">{showMemory(group)}</span>
      </div>
      {/*
        Полоса меряется наибольшим в списке, а не сотней процентов и не всей
        памятью машины. Виджет отвечает на вопрос «кто ест», а не «сколько
        осталось»: в покое восемь полос от сотни были бы восемью пустыми
        треками. Верхняя строка поэтому всегда полная — это шкала «во сколько
        раз больше соседа», и числом она не притворяется: число стоит рядом.
      */}
      <Bar fill={peak > 0 ? amount(group, mode) / peak : null} thickness="thin" />
    </li>
  );
}

/** Исходный Classic: разметка и расчёт видимых строк сохранены. */
function AppProcesses({ rows, mode, peak, setMode }: {
  rows: Group[];
  mode: Mode;
  peak: number;
  setMode: (mode: Mode) => void;
}) {
  const t = useText();

  const list = useRef<HTMLUListElement>(null);
  const sample = useRef<HTMLLIElement>(null);
  const space = useHeight(list);
  const rowHeight = useHeight(sample);
  const geometry = useScale();

  /*
   * Строк ровно столько, сколько влезло: обрезанная пополам строка читается
   * как поломка, а лишняя высота внизу — как забытое место. Восьмёрка от Rust
   * остаётся потолком, ниже одной строки не опускаемся.
   *
   * Высота списка от числа строк не зависит (`flex: 1` в карточке), так что
   * замер сам себя не гоняет по кругу.
   */
  const pitch = rowHeight + ROW_GAP * geometry;
  const fits = pitch > 0 ? Math.floor((space + ROW_GAP * geometry) / pitch) : rows.length;
  const shown = rows.slice(0, Math.max(Math.min(fits, rows.length), 1));

  return (
    <div className="widget">
      {/* Цвет меняется вместе с метрикой: жёлтый процессора, бирюза памяти.
          Правило проекта соблюдено — цвет закреплён за метрикой, а метрику
          тут выбирают. */}
      <GlassCard accent={mode === "cpu" ? "cpu" : "ram"}>
        <div className="proc">
          {rows.length === 0 ? (
            /* Между открытием окна и первым измерением проходит доля секунды:
               проценты считаются от прошлого опроса, и первый заход пустой */
            <p className="proc__empty">{t("processes.collecting")}</p>
          ) : (
            <ul className="proc__list" ref={list}>
              {shown.map((group, index) => (
                <Row
                  key={group.name}
                  group={group}
                  mode={mode}
                  peak={peak}
                  measured={index === 0 ? sample : undefined}
                />
              ))}
            </ul>
          )}

          {/*
            Переключатель внизу, как кнопка очистки у буфера обмена: список —
            содержимое карточки, и отдавать ему верхнюю строку под управление
            незачем. Завершения процессов тут нет и не будет.
          */}
          <div className="proc__foot">
            <Toggle
              label="CPU"
              accent="cpu"
              active={mode === "cpu"}
              onChange={() => setMode("cpu")}
            />
            <Toggle
              label="RAM"
              accent="ram"
              active={mode === "ram"}
              onChange={() => setMode("ram")}
            />
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

/** Вариант A: обе метрики и прокрутка восьмёрки только в NEW. */
function ReferenceProcesses({ rows, mode, peak, setMode }: {
  rows: Group[];
  mode: Mode;
  peak: number;
  setMode: (mode: Mode) => void;
}) {
  const t = useText();
  const viewport = useRef<HTMLDivElement>(null);
  const sample = useRef<HTMLLIElement>(null);
  const space = useHeight(viewport);
  const rowHeight = useHeight(sample);
  // Показываем целые строки; остальные остаются в списке и доступны прокруткой.
  const height = rowHeight > 0
    ? Math.min(space, Math.max(1, Math.floor(space / rowHeight)) * rowHeight)
    : space;

  return (
    <div className="widget proc-widget--reference">
      <GlassCard accent={mode === "cpu" ? "cpu" : "ram"} flush>
        <div className="proc-ref" data-mode={mode}>
          <div className="proc-ref__columns" aria-hidden="true">
            <span>{t("processes.name")}</span>
            <span className="proc-ref__column" data-active={mode === "cpu"}>
              CPU{mode === "cpu" ? " ↓" : ""}
            </span>
            <span className="proc-ref__column" data-active={mode === "ram"}>
              RAM{mode === "ram" ? " ↓" : ""}
            </span>
          </div>
          <div className="proc-ref__viewport" ref={viewport}>
            {rows.length === 0 ? (
              <p className="proc-ref__empty">{t("processes.collecting")}</p>
            ) : (
              <ul
                className="proc-ref__list"
                style={{ height: height > 0 ? height : "100%" }}
                tabIndex={0}
                aria-label={t("widget.processes")}
                onMouseDown={(event) => {
                  // Ползунок прокрутки держит мышь; подписи всё ещё двигают окно.
                  const list = event.currentTarget;
                  const scrollbar = list.offsetWidth - list.clientWidth;
                  if (scrollbar > 0 && event.clientX >= list.getBoundingClientRect().right - scrollbar) {
                    event.stopPropagation();
                  }
                }}
              >
                {rows.map((group, index) => (
                  <li className="proc-ref__item" key={group.name} ref={index === 0 ? sample : undefined}>
                    <div className="proc-ref__identity" title={group.name}>
                      <span className="proc-ref__name">{group.name}</span>
                      {group.count > 1 && <span className="proc-ref__count">×{group.count}</span>}
                    </div>
                    <span className="proc-ref__cpu tabular" data-active={mode === "cpu"}>
                      <span className="proc-ref__metric-label">CPU</span>
                      <span>{showCpu(group)}</span>
                    </span>
                    <span className="proc-ref__ram tabular" data-active={mode === "ram"}>
                      <span className="proc-ref__metric-label">RAM</span>
                      <span>{showMemory(group)}</span>
                    </span>
                    <div className="proc-ref__bar">
                      <Bar fill={peak > 0 ? amount(group, mode) / peak : null} thickness="thin" />
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="proc-ref__foot">
            <Toggle label="CPU" active={mode === "cpu"} onChange={() => setMode("cpu")} />
            <Toggle label="RAM" active={mode === "ram"} onChange={() => setMode("ram")} />
            <span className="proc-ref__sort-label">{t("processes.sort")}</span>
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

export function ProcessesWidget() {
  const { effectiveDesign } = useLook();
  const { byCpu, byMemory } = useProcesses();
  // Состояние принадлежит виджету, поэтому смена дизайна не сбрасывает режим.
  const [mode, setMode] = useState<Mode>("cpu");
  const rows = mode === "cpu" ? byCpu : byMemory;
  const peak = rows[0] ? amount(rows[0], mode) : 0;
  const props = { rows, mode, peak, setMode };
  return effectiveDesign === "reference"
    ? <ReferenceProcesses {...props} />
    : <AppProcesses {...props} />;
}
