import { useRef } from "react";
import { useLook } from "../../lib/look";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import { DisksReference } from "./DisksReference";
import { useTelemetry } from "../../lib/telemetry";
import { capacityOf, disks, type Disk } from "../../lib/sensors";
import { spaceAccent } from "../../lib/accent";
import { formatReading, NO_VALUE, shareOf, showReading } from "../../lib/format";
import { useContentHeight } from "../../lib/grow";
import { useSettings } from "../../lib/settings";
import { GlassCard } from "../../components/GlassCard";
import { Bar } from "../../components/Bar";
import { MetricLabel } from "../../components/MetricLabel";
import { DriveIcon } from "../../components/Icons";
import "./sections.css";
import { useText } from "../../lib/i18n";

function DiskRow({ disk }: { disk: Disk }) {
  const free = showReading(disk.free);
  const total = showReading(disk.total);
  const life = showReading(disk.life);
  const capacity = capacityOf(disk.name);

  // Все измеряемые температуры подряд; пороги Warning и Critical отсеяны
  const temperatures = disk.temperatures
    .map((reading) => formatReading(reading.value, reading.unit))
    .join(" · ");

  return (
    <div className="disk">
      {/*
        Имя и объём — в двух видах сразу: полное для просторной карточки и
        короткое для тесной. Порог показывает одно вместо другого, а текст при
        этом остаётся в разметке, а не уезжает в CSS.
      */}
      <div className="disk__head">
        <span className="disk__name">
          <span className="disk__full"><MetricLabel icon={<DriveIcon />} badge>{disk.name}</MetricLabel></span>
          {capacity !== undefined && <span className="disk__brief"><MetricLabel icon={<DriveIcon />}>{capacity}</MetricLabel></span>}
        </span>
        <span className="disk__size tabular">
          <span className="disk__full">
            {free && total ? `${free} of ${total}` : NO_VALUE}
          </span>
          {/* В тесной карточке остаётся одно свободное место: «из всего» и так
              стоит слева объёмом накопителя */}
          <span className="disk__brief">{free ?? NO_VALUE}</span>
        </span>
      </div>
      {/*
        Шкала по объёму: ресурс из SMART идёт отдельным числом.

        Цвет ей даёт остаток места, а не метрика накопителя, — полоса и
        читает его через var(--accent) с обёртки.
      */}
      <div data-accent={spaceAccent(disk.usedShare?.value)}>
        <Bar fill={shareOf(disk.usedShare)} />
      </div>
      {/*
        Три части в одной строке: скорости слева, температуры посередине,
        ресурс справа. Прежде температуры стояли отдельной строкой под ними и
        стоили карточке лишней строки на каждый накопитель.
      */}
      <div className="disk__rates tabular">
        <span className="disk__rate">
          {/* R и W вместо слов всегда: строка тут самая тесная в карточке, а
              «чтение» с «записью» стоили ей восьмидесяти точек на ровном месте */}
          R{" "}
          {/* У некоторых NVMe скоростей LHM не отдаёт — здесь будет прочерк */}
          <strong>{showReading(disk.readRate) ?? NO_VALUE}</strong> W{" "}
          <strong>{showReading(disk.writeRate) ?? NO_VALUE}</strong>
        </span>
        {/*
          Температур или ресурса у диска может не быть вовсе — у внешних и
          у старых HDD. Тогда уходит своя часть строки, а не встаёт прочерк
          (этап 57): прочерк значит «датчик есть, значения нет».
        */}
        {disk.temperatures.length > 0 && (
          <span className="disk__rate disk__rate--heat">
            <strong>{temperatures}</strong>
          </span>
        )}
        {life !== undefined && (
          <span className="disk__rate">
            <span className="disk__full">life </span>
            <strong>{life}</strong>
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * Накопители: по строке на каждый показанный.
 *
 * Какие показывать, выбирают на странице виджета в настройках (этап 53):
 * хранятся скрытые, поэтому новый диск появляется сам. Окно встаёт высотой
 * ровно по числу строк — реестровый размер снят под два диска.
 */
export function DisksSection() {
  const state = useTelemetry();
  const t = useText();
  const { hiddenDisks } = useSettings();
  const list = useRef<HTMLDivElement>(null);
  const gpt = useLook().effectiveDesign === "reference";
  const view = useWidgetView();
  const size = useWidgetSize();

  const found = disks(state);
  const shown = found.filter((disk) => !hiddenDisks.includes(disk.name));

  // Подгоняем, только когда есть что показать: строка ожидания или «дисков
  // нет» сжала бы окно в полоску, и при первых же данных его пришлось бы
  // растягивать обратно
  useContentHeight(list, shown.length > 0, `${gpt}:${view}:${size}:${shown.map((disk) => disk.name).join("\n")}`, true);

  return (
    <GlassCard accent="storage">
      {found.length === 0 ? (
        <span className="disk__rates">{t("hardware.noDrives")}</span>
      ) : shown.length === 0 ? (
        // Все найденные скрыты: пустая карточка выглядела бы поломкой
        <span className="disk__rates">{t("hardware.drivesHidden")}</span>
      ) : (
        <div ref={list}>
          {gpt ? <DisksReference disks={shown} ring={view === 1} size={size} /> : shown.map((disk) => <DiskRow key={disk.name} disk={disk} />)}
        </div>
      )}
    </GlassCard>
  );
}
