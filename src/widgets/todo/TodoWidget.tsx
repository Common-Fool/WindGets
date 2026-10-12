import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useWindowDrag } from "../../lib/drag";
import { useText } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { useWidgetLayout } from "../../lib/view";
import { copyToClipboard } from "../../lib/copy";
import { loadTasks, saveTasks, type Board, type Task } from "../../lib/todo";
import { useUndo } from "../../lib/undo";
import { GlassCard } from "../../components/GlassCard";
import { Dots } from "../../components/Dots";
import { Linked } from "../../components/Linked";
import { Undo } from "../../components/Undo";
import {
  BoxCheckedIcon,
  BoxIcon,
  CheckIcon,
  CopyIcon,
  DoneIcon,
  EraseIcon,
  FailedIcon,
} from "../../components/Icons";
import "./todo.css";
import "./todo-reference.css";

/** Сколько держится ответ кнопки копирования. Столько же, сколько у заметок. */
const COPIED_FOR = 1500;

/** Больше списков не заводим: тот же довод, что у страниц заметок. */
const MAX_LISTS = 6;

/** Пустая доска: с ней карточка живёт, пока не пришёл ответ с диска. */
const NOTHING: Board = { lists: [], active: 0 };

/**
 * Номер новому делу: наибольший из имеющихся плюс единица.
 *
 * Не часы: два дела, заведённые в одну миллисекунду, получили бы один номер, а
 * номер тут ключ строки в React и цель переключения и удаления.
 */
function nextId(tasks: Task[]): number {
  return tasks.reduce((max, task) => Math.max(max, task.id), 0) + 1;
}

/** Список для буфера обмена: разметка галочек понятна где угодно. */
function asText(tasks: Task[]): string {
  return tasks.map((task) => `- [${task.done ? "x" : " "}] ${task.text}`).join("\n");
}

export function TodoWidget() {
  useWindowDrag();
  const t = useText();
  const reference = useLook().effectiveDesign === "reference";
  const layout = useWidgetLayout();

  const [board, setBoard] = useState<Board>(NOTHING);
  const [input, setInput] = useState("");
  /**
   * Доска прочитана, и в неё можно дописывать.
   *
   * Отдельным состоянием, а не проверкой `stored.current` прямо в разметке:
   * ссылка перерисовку не вызывает, и поле осталось бы серым до первой чужой
   * причины перерисоваться.
   */
  const [ready, setReady] = useState(false);
  /**
   * Что не так с диском.
   *
   * Состояния два: не прочиталось — на диске может лежать целая доска, и
   * писать нельзя вовсе; не записалось — в карточке изменение есть, а на диске
   * осталось прежнее.
   */
  const [alarm, setAlarm] = useState<"read" | "write" | null>(null);
  /** Ответ кнопки копирования: `undefined` — её ещё не нажимали. */
  const [copied, setCopied] = useState<boolean | undefined>(undefined);
  /**
   * Доска до последней очистки. Пока она тут, очистку можно отменить.
   *
   * Отката у списка дел нет — это не поле ввода, и Ctrl+Z ему не помощник, — а
   * очистка сносит работу целого дня одним нажатием.
   */
  const { kept: undo, remember, forget } = useUndo<Board>();
  /** Номер дела, которое правят прямо сейчас. */
  const [editing, setEditing] = useState<number | null>(null);
  /** Правленый текст этого дела: в самом списке он меняется только по Enter. */
  const [draft, setDraft] = useState("");

  /**
   * Что лежит на диске прямо сейчас.
   *
   * ВАЖНО: `null` означает «ещё не читали», и писать в этом состоянии нельзя,
   * чтобы не затереть доску прошлых запусков.
   */
  const stored = useRef<Board | null>(null);
  /** То же, что в `board`, но доступное из обещаний и обработчиков. */
  const latest = useRef<Board>(NOTHING);
  /** Доска последней начатой записи: по ней узнаётся отставший ответ. */
  const wanted = useRef<Board | null>(null);

  /**
   * Кладёт доску на диск.
   *
   * ВАЖНО: **неудачная запись возвращает `stored` назад,** иначе повторить ту
   * же попытку было бы нечем. Отвечает только самая свежая попытка (`wanted`):
   * две записи внахлёст бывают, и отставшая, упав позже успевшей, сдвинула бы
   * `stored` в прошлое.
   */
  const keep = useCallback((next: Board) => {
    const before = stored.current;
    if (before === null) return;

    stored.current = next;
    wanted.current = next;

    saveTasks(next).then(
      () => {
        if (wanted.current === next) setAlarm(null);
      },
      () => {
        if (wanted.current !== next) return;
        stored.current = before;
        setAlarm("write");
      },
    );
  }, []);

  useEffect(() => {
    let alive = true;

    loadTasks().then(
      (kept) => {
        if (!alive) return;
        stored.current = kept;
        latest.current = kept;
        setBoard(kept);
        setReady(true);
      },
      () => {
        // Не прочиталось — не пишем вовсе: поверх нечитаемого файла может
        // лежать целая доска. `stored` остаётся пустым, и это и есть запрет
        if (alive) setAlarm("read");
      },
    );

    return () => {
      alive = false;
    };
  }, []);

  /*
   * Меню строки дела: его показывает Rust, а выбранный пункт возвращается сюда
   * событием. Нативным оно сделано по той же причине, что и меню виджета: окно
   * бывает высотой в сотню точек, и всплывающему меню её границы не указ.
   *
   * Слушатель ставится один раз на окно. Обработчики внутри зовут функции,
   * замкнутые на первый заход отрисовки, — поэтому все они работают со
   * ссылками, а не с состоянием.
   */
  useEffect(() => {
    const stop = listen<{ action: string; id: number }>("task-menu", (event) => {
      const { action, id } = event.payload;

      if (action === "edit") startEdit(id);
      if (action === "remove") drop(id);
    });

    return () => {
      void stop.then((off) => off());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const answer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(answer.current), []);

  /*
   * Пустая доска показывается одним пустым списком: список без дел — это
   * список, в который ещё не писали. Сам список заводится при первом деле, а не
   * при открытии окна.
   */
  const tasks = board.lists[board.active] ?? [];
  const alone = board.lists.length <= 1;
  const finished = tasks.some((task) => task.done);

  /**
   * Одна дверь на все правки доски: пишем сразу, задержки тут не нужно.
   *
   * ВАЖНО: **любая правка снимает предложение вернуть очистку.** Снимок хранит
   * доску целиком, и возврат поверх новых дел стёр бы их, а заодно перебросил
   * бы на тот список, который был открыт до очистки. Возврат осмыслен ровно до
   * следующего действия — дальше он не отмена, а откат вслепую.
   */
  function apply(next: Board) {
    forget();
    latest.current = next;
    setBoard(next);
    keep(next);
  }

  /** Правит открытый список, заводя его, если доска пуста. */
  function edit(change: (tasks: Task[]) => Task[]) {
    const now = latest.current;
    const lists = now.lists.length === 0 ? [[]] : now.lists;
    const at = Math.min(now.active, lists.length - 1);

    apply({ lists: lists.map((list, i) => (i === at ? change(list) : list)), active: at });
  }

  function rollback() {
    if (undo === null) return;
    apply(undo);
  }

  /*
   * Запрет «не прочитано — не пишем» стоит по ссылке, а не по состоянию
   * `ready`: эти же функции зовёт слушатель меню, заведённый один раз при
   * сборке, и состояние в нём навсегда осталось бы тем, каким было тогда.
   */
  function add(text: string) {
    if (stored.current === null) return;
    edit((list) => [...list, { id: nextId(list), text, done: false }]);
  }

  function toggle(id: number) {
    if (stored.current === null) return;
    edit((list) => list.map((task) => (task.id === id ? { ...task, done: !task.done } : task)));
  }

  function drop(id: number) {
    if (stored.current === null) return;
    edit((list) => list.filter((task) => task.id !== id));
  }

  /** Открывает строку на правку: текст уходит в поле, список его не меняет. */
  function startEdit(id: number) {
    const task = (latest.current.lists[latest.current.active] ?? []).find(
      (one) => one.id === id,
    );
    if (task === undefined) return;

    setDraft(task.text);
    setEditing(id);
  }

  /**
   * Записывает правку.
   *
   * Пустой текст правкой не считается: дело без слов — это не дело, а удалять
   * его молча, когда человек всего лишь стёр строку, было бы неожиданностью.
   * Такая правка просто отменяется.
   */
  function commitEdit(id: number, text: string) {
    setEditing(null);

    const said = text.trim();
    if (said.length === 0) return;

    edit((list) => list.map((task) => (task.id === id ? { ...task, text: said } : task)));
  }

  function pick(at: number) {
    apply({ ...latest.current, active: at });
  }

  function addList() {
    const now = latest.current;
    const lists = [...(now.lists.length === 0 ? [[]] : now.lists), []];
    if (lists.length > MAX_LISTS) return;

    apply({ lists, active: lists.length - 1 });
  }

  function copy() {
    void copyToClipboard(asText(tasks)).then((done) => {
      setCopied(done);
      window.clearTimeout(answer.current);
      answer.current = window.setTimeout(() => setCopied(undefined), COPIED_FOR);
    });
  }

  function clearDone() {
    const before = latest.current;
    edit((list) => list.filter((task) => !task.done));
    remember(before);
  }

  /*
   * Одна кнопка на два действия, по порядку: сперва вынести дела, а у уже
   * пустого списка — убрать сам список. Так удаление списка стоит двух
   * раздельных нажатий, и первое из них возвращается кнопкой отмены.
   */
  function clearAll() {
    if (tasks.length > 0) {
      const before = latest.current;
      edit(() => []);
      remember(before);
      return;
    }

    if (alone) return;

    const now = latest.current;
    const lists = now.lists.filter((_, at) => at !== now.active);
    apply({ lists, active: Math.min(now.active, lists.length - 1) });
  }

  return (
    <div className={reference ? "widget widget--todo-reference" : "widget"}>
      {/* Акцент нейтральный: дела не железо, метрики у них нет */}
      <GlassCard accent="neutral">
        <div className={reference ? "todo todo--reference" : "todo"} data-layout={layout} data-alarm={alarm !== null}>
          <Dots
            scrollable={reference}
            count={board.lists.length}
            active={board.active}
            limit={MAX_LISTS}
            onPick={pick}
            onAdd={addList}
            disabled={reference && !ready}
          />

          {reference && (
            <span className="todo__summary" title={t("todo.progress", { done: tasks.filter((task) => task.done).length, total: tasks.length })}>
              {tasks.filter((task) => task.done).length} / {tasks.length}
            </span>
          )}

          {tasks.length === 0 ? (
            <p className="todo__empty">{reference && !ready ? alarm === "read" ? "" : t("common.loading") : t("todo.empty")}</p>
          ) : (
            <ul className="todo__list">
              {tasks.map((task) => (
                /*
                 * Строка дела — не кнопка: за кнопку окно не потащишь, а тянут
                 * виджеты за что угодно, кроме органов управления. Крестик
                 * удаления стоит отдельной кнопкой рядом: кнопка в кнопке
                 * недопустима, да и нажатие по нему всплывало бы в переключение.
                 */
                <li
                  key={task.id}
                  className="todo__item"
                  data-done={task.done}
                  /*
                   * Двойное нажатие по строке открывает её на правку. Галочку
                   * оно не трогает вовсе: переключает её только сам квадратик,
                   * и это не мелочь — пока переключала вся строка, каждое
                   * двойное нажатие успевало перевернуть дело, а обработчику
                   * приходилось переворачивать его обратно.
                   */
                  onDoubleClick={() => {
                    if (editing !== task.id) startEdit(task.id);
                  }}
                  onContextMenu={(event) => {
                    /*
                     * ВАЖНО: **в поле правки меню строки не всплывает.** Там
                     * правая кнопка — это «Вырезать / Копировать / Вставить»
                     * движка, без которых в строку нечем вставить текст мышью.
                     * Событие уходит дальше нетронутым: гасит его оболочка
                     * окна, и она же оставляет полю его собственное меню.
                     */
                    if (event.target instanceof Element && event.target.closest("input")) {
                      return;
                    }

                    // Меню строки, а не виджета: общее живёт по краям окна, и
                    // пускать сюда оба разом нельзя — у края они совпали бы
                    event.preventDefault();
                    event.stopPropagation();
                    void invoke("open_task_menu", {
                      x: event.clientX,
                      y: event.clientY,
                      id: task.id,
                    }).catch(() => {
                      // Вне Tauri звать некого
                    });
                  }}
                >
                  {/*
                    Квадратик — кнопка, и переключает дело только он. Двойное
                    нажатие по нему до строки не доходит: иначе оно открывало бы
                    правку заодно с двумя переключениями подряд.
                  */}
                  <button
                    type="button"
                    className="todo__check"
                    title={task.done ? t("todo.markUndone") : t("todo.markDone")}
                    onClick={() => toggle(task.id)}
                    onDoubleClick={(event) => event.stopPropagation()}
                  >
                    {task.done ? <BoxCheckedIcon /> : <BoxIcon />}
                  </button>

                  {editing === task.id ? (
                    <span className={reference ? "todo__edit-box" : "todo__edit-plain"}>
                      {reference && <span className="todo__text todo__edit-ghost" aria-hidden="true">{task.text}</span>}
                      <input
                        type="text"
                        className="input todo__edit"
                        value={draft}
                        autoFocus
                        spellCheck={false}
                        onChange={(event) => setDraft(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") commitEdit(task.id, draft);
                          // Побег без правки: строка остаётся какой была
                          if (event.key === "Escape") setEditing(null);
                        }}
                        /* Уход из поля правку записывает: щелчок мимо — обычный
                           способ закончить, и терять на нём написанное нельзя */
                        onBlur={() => commitEdit(task.id, draft)}
                      />
                    </span>
                  ) : (
                    <span className="todo__text" title={task.text}>
                      <Linked text={task.text} />
                    </span>
                  )}

                  <button
                    type="button"
                    className="todo__drop"
                    title={t("common.remove")}
                    onClick={(event) => {
                      event.stopPropagation();
                      drop(task.id);
                    }}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}

          {/* Поле и значки одной строкой: вторая отняла бы у списка ещё одно дело */}
          <div className="todo__foot">
            <input
              type="text"
              className="input todo__input"
              placeholder={t("todo.placeholder")}
              value={input}
              disabled={!ready}
              spellCheck={false}
              onChange={(event) => setInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.nativeEvent.isComposing) return;

                const said = input.trim();
                if (said.length === 0) return;

                event.preventDefault();
                add(said);
                setInput("");
              }}
            />

            <button
              type="button"
              className="todo__act"
              title={copied === undefined ? t("todo.copy") : copied ? t("common.copied") : t("common.failed")}
              disabled={tasks.length === 0}
              onClick={copy}
            >
              {copied === undefined ? <CopyIcon /> : copied ? <DoneIcon /> : <FailedIcon />}
            </button>

            <button
              type="button"
              className="todo__act"
              title={t("todo.clearDone")}
              disabled={!finished}
              onClick={clearDone}
            >
              <CheckIcon />
            </button>

            <button
              type="button"
              className="todo__act"
              title={tasks.length > 0 ? t("todo.clearAll") : t("todo.removeList")}
              disabled={tasks.length === 0 && alone}
              onClick={clearAll}
            >
              <EraseIcon />
            </button>
          </div>

          {/* Нижняя строка появляется только тогда, когда есть беда */}
          {alarm !== null && (
            <div className="todo__note">
              <span className="todo__trouble">
                {alarm === "read" ? t("todo.cantRead") : t("common.notSaved")}
              </span>
            </div>
          )}

          {/* Возврат стоит поверх содержимого, посреди карточки */}
          {undo !== null && <Undo onClick={rollback} />}
        </div>
      </GlassCard>
    </div>
  );
}
