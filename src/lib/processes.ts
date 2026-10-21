import { liveSnapshot } from "./live";

export type Group = {
  /** Имя без расширения: «msedge», а не «msedge.exe». */
  name: string;
  /** Проценты от всей машины: сотня — все ядра заняты целиком. */
  cpu: number;
  /** Рабочее множество в байтах, сумма по группе. */
  memory: number;
  /** Сколько процессов с этим именем. Единица в строке не показывается. */
  count: number;
};

/**
 * Оба порядка приходят разом: переключатель ЦПУ/ОЗУ выбирает из уже
 * присланного, а не спрашивает заново. Через команду он ждал бы до двух
 * секунд — столько идёт шаг опроса.
 */
export type ProcessesState = {
  byCpu: Group[];
  byMemory: Group[];
};

const EMPTY: ProcessesState = { byCpu: [], byMemory: [] };

/*
 * Подписка одна на окно, как у телеметрии и пинга, и снимается, когда
 * слушателей не осталось (`live.ts`).
 */
export const useProcesses = liveSnapshot<ProcessesState>("processes_state", "processes", EMPTY);
