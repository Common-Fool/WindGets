import type { Disk } from "../../lib/sensors";
import { capacityOf } from "../../lib/sensors";
import { spaceAccent } from "../../lib/accent";
import { formatReading, NO_VALUE, shareOf, showReading } from "../../lib/format";
import { Arc } from "../../components/Arc";
import { Bar } from "../../components/Bar";
import { DriveIcon } from "../../components/Icons";
import "./disks-reference.css";

function Quantity({ text, main = false }: { text: string; main?: boolean }) {
  const split = text.lastIndexOf(" ");
  return <span className={main ? "disks-ref__quantity" : ""}>
    {split < 0 ? text : <>{text.slice(0, split)}<small>{text.slice(split + 1)}</small></>}
  </span>;
}

function ReadingPair({ label, value }: { label: string; value: string }) {
  return <span className="disks-ref__pair" data-field={label} title={value}><span>{label}</span><strong>{value}</strong></span>;
}

/** GPT: имя и свободно/всего в заголовке; параметры в одном ряду. */
export function DisksReference({ disks, ring, size }: { disks: Disk[]; ring: boolean; size: number }) {
  const narrow = ring || size !== 0;
  return <div className={`disks-ref${ring ? " disks-ref--ring" : ""}${narrow ? " disks-ref--narrow" : ""}`}>
    {disks.map(disk => {
      const free = showReading(disk.free) ?? NO_VALUE;
      const total = showReading(disk.total) ?? NO_VALUE;
      const life = showReading(disk.life);
      const temperatures = disk.temperatures.map(reading => formatReading(reading.value, reading.unit)).join(" · ");
      const capacity = capacityOf(disk.name);
      const name = narrow && capacity !== undefined
        ? disk.name.replace(/\s*\d+(?:[.,]\d+)?\s*(?:TB|GB|ТБ|ГБ)\s*$/i, "").trim()
        : disk.name;
      const read = showReading(disk.readRate) ?? NO_VALUE;
      const write = showReading(disk.writeRate) ?? NO_VALUE;
      return <div className="disks-ref__disk" key={disk.name} data-accent={spaceAccent(disk.usedShare?.value)}>
        <div className="disks-ref__head">
          <DriveIcon /><span className="disks-ref__name" title={disk.name}>{name}</span>
          {ring && capacity !== undefined && <span className="disks-ref__capacity">{capacity}</span>}
          {!ring && <span className="disks-ref__space tabular">
            <Quantity text={free} main /><span className="disks-ref__total">/{total}</span>
          </span>}
        </div>
        {ring ? <div className="disks-ref__ring-row">
          <Arc value={shareOf(disk.usedShare)} thickness={6.7}>
            <span className="disks-ref__free-label">Free</span><Quantity text={free} main />
          </Arc>
          <div className="disks-ref__ring-values">
            <ReadingPair label="Total" value={total} />
            <ReadingPair label="R" value={read} />
            <ReadingPair label="W" value={write} />
            {life !== undefined && <ReadingPair label="life" value={life} />}
          </div>
        </div> : <>
          <Bar fill={shareOf(disk.usedShare)} thickness="thin" />
          {size !== 2 && <div className="disks-ref__rates tabular">
            <ReadingPair label="R" value={read} /><ReadingPair label="W" value={write} />
            {size === 0 && temperatures !== "" && <ReadingPair label="Temp" value={temperatures} />}
            {life !== undefined && <ReadingPair label="life" value={life} />}
          </div>}
        </>}
      </div>;
    })}
  </div>;
}
