import type { DriverView } from "../../lib/pawnio";
import type { PingState } from "../../lib/ping";
import type { Reading, Roles, SourceState, Unit } from "../../lib/telemetry";
import { GIB, type Answers, type Context } from "./common";

/*
 * Железо превью: процессор, видеокарта, память, накопители и сеть одним
 * снимком телеметрии — как его отдаёт сайдкар. Модели — нейтральные примеры
 * (Ryzen 7 7700X, RTX 5070, NCT6799D), не машина автора.
 */

const MACHINE = "Demo";
const CPU = `${MACHINE}/AMD Ryzen 7 7700X`;
const BOARD = `${MACHINE}/Nuvoton NCT6799D`;
const GPU = `${MACHINE}/NVIDIA GeForce RTX 5070`;
const MEMORY = `${MACHINE}/Total Memory`;
const SSD = `${MACHINE}/Samsung SSD 980 PRO 1TB`;
const SECOND_SSD = `${MACHINE}/WD_BLACK SN850X 2000GB`;
const NET = `${MACHINE}/Ethernet`;

/** Путь, значение, единица; у загрузок — размах колебаний для истории. */
type Row = [path: string, value: number, unit: Unit, swing?: number];

const ROWS: Row[] = [
  [`${CPU}/Load/CPU Total`, 34, "percent", 14],
  [`${CPU}/Temperatures/Core (Tctl/Tdie)`, 63, "celsius"],
  [`${CPU}/Clocks/Cores (Average)`, 4815, "megahertz"],
  [`${CPU}/Powers/Package`, 71.4, "watt"],
  [`${BOARD}/Voltages/Vcore`, 1.236, "volt"],
  [`${BOARD}/Temperatures/Motherboard`, 38, "celsius"],
  [`${BOARD}/Fans/CPU Fan`, 1180, "rpm"],
  [`${BOARD}/Fans/Chassis Fan`, 860, "rpm"],

  [`${GPU}/Load/GPU Core`, 58, "percent", 18],
  [`${GPU}/Load/GPU Memory`, 46, "percent", 6],
  [`${GPU}/Data/GPU Memory Used`, 5.6 * GIB, "byte"],
  [`${GPU}/Data/GPU Memory Total`, 12 * GIB, "byte"],
  [`${GPU}/Temperatures/GPU Core`, 57, "celsius"],
  [`${GPU}/Temperatures/GPU Memory Junction`, 64, "celsius"],
  [`${GPU}/Fans/GPU Fan 1`, 1380, "rpm"],
  [`${GPU}/Fans/GPU Fan 2`, 1360, "rpm"],
  [`${GPU}/Clocks/GPU Core`, 2610, "megahertz"],
  [`${GPU}/Clocks/GPU Memory`, 7000, "megahertz"],
  [`${GPU}/Powers/GPU Package`, 182, "watt"],
  [`${GPU}/Voltages/GPU Core Voltage`, 0.985, "volt"],

  [`${MEMORY}/Load/Memory`, 48, "percent", 4],
  [`${MEMORY}/Data/Memory Used`, 15.3 * GIB, "byte"],
  [`${MEMORY}/Data/Memory Available`, 16.6 * GIB, "byte"],

  [`${SSD}/Load/Used Space`, 62, "percent"],
  [`${SSD}/Data/Free Space`, 354 * GIB, "byte"],
  [`${SSD}/Data/Total Space`, 931 * GIB, "byte"],
  [`${SSD}/Levels/Life`, 97, "percent"],
  [`${SSD}/Throughput/Read Rate`, 42 * 1024 ** 2, "bytePerSecond"],
  [`${SSD}/Throughput/Write Rate`, 6.5 * 1024 ** 2, "bytePerSecond"],
  [`${SSD}/Temperatures/Composite Temperature`, 44, "celsius"],
  [`${SSD}/Temperatures/Temperature #2`, 51, "celsius"],
  [`${SECOND_SSD}/Load/Used Space`, 38, "percent"],
  [`${SECOND_SSD}/Data/Free Space`, 1155 * GIB, "byte"],
  [`${SECOND_SSD}/Data/Total Space`, 1863 * GIB, "byte"],
  [`${SECOND_SSD}/Levels/Life`, 100, "percent"],
  [`${SECOND_SSD}/Throughput/Read Rate`, 0, "bytePerSecond"],
  [`${SECOND_SSD}/Throughput/Write Rate`, 0.4 * 1024 ** 2, "bytePerSecond"],
  [`${SECOND_SSD}/Temperatures/Composite Temperature`, 39, "celsius"],

  [`${NET}/Throughput/Download Speed`, 3.2 * 1024 ** 2, "bytePerSecond", 2.4 * 1024 ** 2],
  [`${NET}/Throughput/Upload Speed`, 420 * 1024, "bytePerSecond", 300 * 1024],
  [`${NET}/Data/Data Downloaded`, 12.4 * GIB, "byte"],
  [`${NET}/Data/Data Uploaded`, 1.1 * GIB, "byte"],
];

const ROLES: Roles = {
  cpu: {
    load: `${CPU}/Load/CPU Total`,
    clock: [`${CPU}/Clocks/Cores (Average)`],
    temperature: `${CPU}/Temperatures/Core (Tctl/Tdie)`,
    power: `${CPU}/Powers/Package`,
    voltage: `${BOARD}/Voltages/Vcore`,
  },
  board: {
    cpuFan: `${BOARD}/Fans/CPU Fan`,
    chassisFan: `${BOARD}/Fans/Chassis Fan`,
    temperature: `${BOARD}/Temperatures/Motherboard`,
  },
  gpu: {
    name: "NVIDIA GeForce RTX 5070",
    vendor: "nvidia",
    load: `${GPU}/Load/GPU Core`,
    memoryLoad: `${GPU}/Load/GPU Memory`,
    memoryUsed: `${GPU}/Data/GPU Memory Used`,
    memoryTotal: `${GPU}/Data/GPU Memory Total`,
    temperature: `${GPU}/Temperatures/GPU Core`,
    secondTemperature: { path: `${GPU}/Temperatures/GPU Memory Junction`, kind: "memory" },
    fans: [`${GPU}/Fans/GPU Fan 1`, `${GPU}/Fans/GPU Fan 2`],
    clock: `${GPU}/Clocks/GPU Core`,
    memoryClock: `${GPU}/Clocks/GPU Memory`,
    power: `${GPU}/Powers/GPU Package`,
    voltage: `${GPU}/Voltages/GPU Core Voltage`,
  },
};

/** Сколько снимков засевается в историю: на ряд столбиков с запасом. */
const HISTORY = 60;

/** Снимок номер `step` из `HISTORY`: последний — ровно значения из таблицы. */
function snapshot(step: number): SourceState {
  const phase = HISTORY - 1 - step;
  const readings: Reading[] = ROWS.map(([path, value, unit, swing]) => ({
    path,
    name: path.slice(path.lastIndexOf("/") + 1),
    sensorId: path,
    kind: path.split("/")[2] ?? "",
    value: swing === undefined ? value : Math.max(0, value + swing * Math.sin(phase / 3.1) * Math.cos(phase / 7.3)),
    unit,
  }));
  return { status: "ok", snapshot: { at: step + 1, readings, roles: ROLES, choices: { fans: [], temperatures: [], voltages: [] } } };
}

let seeded = false;

/*
 * История столбиков копится из событий, а у превью опроса нет: сразу после
 * первого ответа отыгрываем пачку снимков, будто карточка простояла пять
 * минут. Ряды истории начинают копиться, когда карточка их попросила, — то
 * есть после первой отрисовки, отсюда задержка.
 */
function seed(context: Context): void {
  if (seeded) return;
  seeded = true;
  window.setTimeout(() => {
    for (let step = 0; step < HISTORY; step++) context.emit("telemetry", snapshot(step));
  }, 300);
}

const PING: PingState = {
  probes: [
    { name: "Cloudflare", target: "1.1.1.1", reading: { latency: 11, jitter: 1.4, loss: 0 } },
    { name: "Google", target: "8.8.8.8", reading: { latency: 14, jitter: 2.1, loss: 0 } },
    { name: "Quad9", target: "9.9.9.9", reading: { latency: 19, jitter: 3.6, loss: 0 } },
  ],
};

const PAWNIO: DriverView = {
  status: "ready",
  version: "2.0.0",
  ours: false,
  job: null,
  failure: null,
  downloadBytes: 0,
  page: "",
};

export const hardware: Answers = {
  telemetry_state: (_args, context) => {
    seed(context);
    return snapshot(HISTORY - 1);
  },
  ping_state: () => PING,
  pawnio_state: () => PAWNIO,
};
