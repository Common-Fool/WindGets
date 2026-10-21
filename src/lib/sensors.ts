import { findByPathEnd, findReading, type Reading, type Roles, type SourceState, type Unit } from "./telemetry";

/*
 * Именованные селекторы датчиков.
 *
 * Все ищут по хвосту пути: первый сегмент полного пути — имя машины, и
 * завязываться на него нельзя. Индексы датчиков не используются нигде,
 * только имена — LHM их порядок не гарантирует.
 *
 * Пути сопоставляются без первого сегмента — имени машины.
 */

/** Второй сегмент пути — имя железа; первый занят именем машины. */
function hardwareOf(path: string): string {
  return path.split("/")[1] ?? "";
}

/** Датчик внутри конкретного железа: так не спутаешь два одинаковых имени. */
function inHardware(
  state: SourceState,
  hardware: string,
  suffix: string,
): Reading | undefined {
  if (state.status !== "ok") return undefined;
  return state.snapshot.readings.find(
    (reading) => hardwareOf(reading.path) === hardware && reading.path.endsWith(suffix),
  );
}


/* ===================== Роли: процессор, плата, видеокарта ===================== */

/*
 * Процессор, плата и видеокарта ищутся не хвостами, а по ролям, которые
 * считает Rust (`lhm/roles.rs`, этап 57): сперва железо, потом датчик внутри
 * него, с запасными именами под Intel, AMD и NVIDIA. Хвост по всему дереву
 * на машине со встройкой и дискретной видеокартой брал бы первую по порядку.
 *
 * Договор у всех селекторов ниже один:
 * - `undefined` — роли нет, датчика на этом железе не бывает, и ячейка уходит;
 * - показание с `value: null` — роль есть, значения сейчас нет, и ячейка
 *   пишет «—». Сюда же попадает датчик, пропавший из снимка на такт: Rust
 *   помнит роли до выхода, и ряд не исчезает под рукой.
 */

function rolesOf(state: SourceState): Roles | undefined {
  return state.status === "ok" ? state.snapshot.roles : undefined;
}

/** Показание по пути роли; пропавшее из снимка — пустое, но на месте. */
function byRole(state: SourceState, path: string | null | undefined, unit: Unit): Reading | undefined {
  if (path == null) return undefined;
  return (
    findReading(state, path) ?? {
      path,
      name: path.slice(path.lastIndexOf("/") + 1),
      sensorId: "",
      kind: "",
      value: null,
      unit,
    }
  );
}

/* ========================= Процессор ========================= */

export const cpuLoad = (state: SourceState) => byRole(state, rolesOf(state)?.cpu.load, "percent");
export const cpuTemp = (state: SourceState) => byRole(state, rolesOf(state)?.cpu.temperature, "celsius");
export const cpuPower = (state: SourceState) => byRole(state, rolesOf(state)?.cpu.power, "watt");
/** Напряжение ядра: у процессора или, у Zen 4 и новее, у чипа платы. */
export const cpuVoltage = (state: SourceState) => byRole(state, rolesOf(state)?.cpu.voltage, "volt");

/**
 * Частота ядер. У AMD это готовая `Cores (Average)`, у Intel средней в
 * выхлопе нет — считаем её по частотам ядер сами.
 */
export function cpuClock(state: SourceState): Reading | undefined {
  const paths = rolesOf(state)?.cpu.clock ?? [];
  if (paths.length === 0) return undefined;
  if (paths.length === 1) return byRole(state, paths[0], "megahertz");

  const values = paths
    .map((path) => findReading(state, path)?.value)
    .filter((value): value is number => value != null);
  return {
    path: paths[0]!,
    name: "Cores (Average)",
    sensorId: "",
    kind: "Clock",
    value: values.length > 0 ? values.reduce((sum, value) => sum + value, 0) / values.length : null,
    unit: "megahertz",
  };
}

/* ==================== Плата и корпус ==================== */

export const cpuFan = (state: SourceState) => byRole(state, rolesOf(state)?.board.cpuFan, "rpm");
export const chassisFan = (state: SourceState) => byRole(state, rolesOf(state)?.board.chassisFan, "rpm");
export const boardTemp = (state: SourceState) => byRole(state, rolesOf(state)?.board.temperature, "celsius");

/* ========================= Видеокарта ========================= */

/**
 * Есть ли дискретная видеокарта: `undefined` — снимка ещё нет, судить рано;
 * `false` — только встройка или ничего, и карточке показывать нечего.
 */
export function gpuPresent(state: SourceState): boolean | undefined {
  const roles = rolesOf(state);
  return roles === undefined ? undefined : roles.gpu !== null;
}

const gpuRoles = (state: SourceState) => rolesOf(state)?.gpu ?? undefined;

export const gpuLoad = (state: SourceState) => byRole(state, gpuRoles(state)?.load, "percent");
export const gpuTemp = (state: SourceState) => byRole(state, gpuRoles(state)?.temperature, "celsius");
/**
 * Вторая температура: память, а без неё hot spot. Hotspot у RTX 50 NVIDIA
 * закрыла в NVAPI, у карт без GDDR6X нет температуры памяти.
 */
export const gpuSecondTemp = (state: SourceState) =>
  byRole(state, gpuRoles(state)?.secondTemperature?.path, "celsius");
export const gpuSecondTempKind = (state: SourceState) => gpuRoles(state)?.secondTemperature?.kind;
export const gpuVramUsed = (state: SourceState) => byRole(state, gpuRoles(state)?.memoryUsed, "byte");
export const gpuVramTotal = (state: SourceState) => byRole(state, gpuRoles(state)?.memoryTotal, "byte");
export const gpuVramLoad = (state: SourceState) => byRole(state, gpuRoles(state)?.memoryLoad, "percent");
export const gpuClock = (state: SourceState) => byRole(state, gpuRoles(state)?.clock, "megahertz");
export const gpuMemoryClock = (state: SourceState) => byRole(state, gpuRoles(state)?.memoryClock, "megahertz");
export const gpuPower = (state: SourceState) => byRole(state, gpuRoles(state)?.power, "watt");
export const gpuVoltage = (state: SourceState) => byRole(state, gpuRoles(state)?.voltage, "volt");
/** Вентиляторы: один у карты с одним кулером, два — у остальных. */
export const gpuFans = (state: SourceState): Reading[] =>
  (gpuRoles(state)?.fans ?? []).flatMap((path) => byRole(state, path, "rpm") ?? []);
/**
 * Частоту памяти NVIDIA LHM отдаёт физическую, а привычная по спецификациям
 * вдвое выше. Как отдаёт её AMD — не проверено, поэтому удваиваем только тут.
 */
export const gpuDoublesMemoryClock = (state: SourceState) => gpuRoles(state)?.vendor === "nvidia";

/* ========================= Память ========================= */

/* Группа «Virtual Memory» с префиксом /vram/ — это файл подкачки, не сюда. */
export const ramLoad = (state: SourceState) => findByPathEnd(state, "/Total Memory/Load/Memory");
export const ramUsed = (state: SourceState) =>
  findByPathEnd(state, "/Total Memory/Data/Memory Used");
export const ramFree = (state: SourceState) =>
  findByPathEnd(state, "/Total Memory/Data/Memory Available");

/** Отдельного датчика «всего» у LHM нет — складываем занятое и доступное. */
export function ramTotal(state: SourceState): number | null {
  const used = ramUsed(state)?.value;
  const free = ramFree(state)?.value;
  return used != null && free != null ? used + free : null;
}

/* ========================= Накопители ========================= */

export type Disk = {
  /** Имя из выхлопа, например «Samsung SSD 980 PRO 1TB». */
  name: string;
  usedShare: Reading | undefined;
  free: Reading | undefined;
  total: Reading | undefined;
  /**
   * Все измеряемые температуры накопителя в порядке выхлопа: сводная,
   * затем датчики по номерам. Пороги `Warning` и `Critical` сюда не идут —
   * это уставки, а не показания.
   */
  temperatures: Reading[];
  /** Остаток ресурса по SMART. */
  life: Reading | undefined;
  readRate: Reading | undefined;
  writeRate: Reading | undefined;
};

/**
 * Измеряемая температура, а не порог срабатывания.
 *
 * У NVMe это сводная `Composite Temperature` и датчики `Temperature #n`, у
 * SATA и HDD — одна `Temperature` без номера (этап 57: прежний фильтр её
 * отсеивал, и у таких дисков температуры не было вовсе). Разница и пороги
 * (`Temperature Difference`, `Warning`, `Critical`) сюда не идут.
 */
function isMeasuredTemperature(reading: Reading): boolean {
  if (!reading.path.includes("/Temperatures/")) return false;
  return reading.name === "Composite Temperature" || /^Temperature( #\d+)?$/.test(reading.name);
}

/**
 * Накопители находим по датчику заполнения — он есть у каждого.
 * Модели не зашиты: добавится третий диск, появится сам.
 */
const USED_SPACE = "/Load/Used Space";

/*
 * Накопитель — по началу пути его датчика заполнения, а не по второму
 * сегменту: косая бывает в самой модели («KINGSTON SV100S2/64G»), и второй
 * сегмент усекал имя, а два диска с общим началом склеивал в один. Путь
 * диска — «машина/модель/группа/датчик», так что модель — всё между первым
 * сегментом и двумя последними.
 */
export function disks(state: SourceState): Disk[] {
  if (state.status !== "ok") return [];
  const readings = state.snapshot.readings;

  const prefixes = readings
    .filter((reading) => reading.path.endsWith(USED_SPACE))
    .map((reading) => reading.path.slice(0, -USED_SPACE.length));

  return prefixes.map((prefix) => {
    const exact = (suffix: string) => readings.find((reading) => reading.path === prefix + suffix);
    // Ровно «группа/датчик» после модели: иначе диск «KINGSTON» забрал бы и
    // датчики соседа «KINGSTON/64G»
    const own = (reading: Reading) =>
      reading.path.startsWith(`${prefix}/`) && reading.path.slice(prefix.length + 1).split("/").length === 2;
    return {
      name: prefix.slice(prefix.indexOf("/") + 1),
      usedShare: exact(USED_SPACE),
      free: exact("/Data/Free Space"),
      total: exact("/Data/Total Space"),
      temperatures: readings.filter((reading) => own(reading) && isMeasuredTemperature(reading)),
      life: exact("/Levels/Life"),
      readRate: exact("/Throughput/Read Rate"),
      writeRate: exact("/Throughput/Write Rate"),
    };
  });
}

/**
 * Объём накопителя из его же модели: «Samsung SSD 970 EVO Plus 2TB» → «2 TB».
 *
 * В самой узкой карточке модель не помещается никак, а объём различает диски
 * не хуже и стоит вчетверо меньше места. Нужен и списку дисков в настройках.
 * Берём именно из имени, а не из `disk.total`: тот у NVMe отдаёт «1000.2 ГБ»,
 * а на коробке написано «1 ТБ».
 *
 * Букв дисков тут быть не может: в выхлопе LibreHardwareMonitor их нет вовсе.
 *
 * Конец единицы — «дальше не буква и не цифра», а не `\b`: граница слова в
 * JavaScript знает только латиницу, и «1 ТБ» с `\b` не находился вовсе (нашло
 * ревью второй модели на этапе 53).
 */
export function capacityOf(name: string): string | undefined {
  const found = name.match(/(\d+(?:[.,]\d+)?)\s*(TB|GB|ТБ|ГБ)(?![\p{L}\p{N}])/giu);
  if (found === null) return undefined;

  const last = found[found.length - 1]!;
  const size = last.match(/[\d.,]+/u)?.[0];
  if (size === undefined) return undefined;

  return `${size} ${/T|Т/iu.test(last) ? "TB" : "GB"}`;
}

/* ========================= Сеть ========================= */

export type Network = {
  name: string;
  download: Reading | undefined;
  upload: Reading | undefined;
  totalDownloaded: Reading | undefined;
  totalUploaded: Reading | undefined;
};

/**
 * Активный адаптер выбираем по объёму скачанного за сессию.
 *
 * По именам искать нельзя: они локализованы («Беспроводная сеть»), да и
 * виртуальных адаптеров в системе несколько, все с нулями.
 */
export function activeNetwork(state: SourceState): Network | undefined {
  if (state.status !== "ok") return undefined;

  let busiest: Reading | undefined;
  for (const reading of state.snapshot.readings) {
    if (!reading.path.endsWith("/Data/Data Downloaded")) continue;
    if (reading.value == null) continue;
    if (busiest?.value == null || reading.value > busiest.value) {
      busiest = reading;
    }
  }

  if (!busiest) return undefined;
  const name = hardwareOf(busiest.path);

  return {
    name,
    download: inHardware(state, name, "/Throughput/Download Speed"),
    upload: inHardware(state, name, "/Throughput/Upload Speed"),
    totalDownloaded: busiest,
    totalUploaded: inHardware(state, name, "/Data/Data Uploaded"),
  };
}
