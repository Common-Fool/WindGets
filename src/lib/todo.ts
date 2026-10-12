import { invoke } from "@tauri-apps/api/core";

/*
 * Список дел: несколько списков в каталоге настроек.
 *
 * Подписки нет по той же причине, что и у заметок: состояние меняет только это
 * окно. Модуль держит `invoke` подальше от карточки.
 */

export type Task = {
  id: number;
  text: string;
  done: boolean;
};

/** Доска: списки дел и тот, что открыт. */
export type Board = {
  lists: Task[][];
  /** Открытый список. Живёт на диске: с ним и открывается окно. */
  active: number;
};

/** Что лежит на диске. Списков ещё не вели — ни одного. */
export function loadTasks(): Promise<Board> {
  return invoke<Board>("todo_lists");
}

/**
 * Кладёт доску на диск.
 *
 * Отказ не глотаем: карточка обязана показать, что список не сохранился.
 */
export function saveTasks(board: Board): Promise<void> {
  return invoke("set_todo_lists", { board });
}
