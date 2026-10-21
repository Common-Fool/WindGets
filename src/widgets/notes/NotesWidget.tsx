import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useWindowDrag } from "../../lib/drag";
import { useLook } from "../../lib/look";
import { useWidgetLayout } from "../../lib/view";
import { useText } from "../../lib/i18n";
import { useDebounced } from "../../lib/debounce";
import { copyToClipboard } from "../../lib/copy";
import { loadNotes, saveNotes, type Book } from "../../lib/notes";
import { useUndo } from "../../lib/undo";
import { GlassCard } from "../../components/GlassCard";
import { Dots } from "../../components/Dots";
import { Linked } from "../../components/Linked";
import { Undo } from "../../components/Undo";
import { CopyIcon, DoneIcon, EraseIcon, FailedIcon, LockIcon, PenIcon } from "../../components/Icons";
import "./notes.css";
import "./notes-reference.css";

/**
 * Через сколько после последней клавиши заметка ложится на диск.
 *
 * Запись неделимая — временный сосед, `sync_all` и переименование, — и на
 * каждую букву её звать незачем. Полсекунды выбраны так, чтобы пауза внутри
 * фразы записью не считалась, а пауза между фразами уже считалась.
 *
 * Отсюда же цена: гибель приложения посреди набора уносит последние полсекунды
 * написанного. Второй заслон против этого — запись по уходу из поля.
 */
const SAVE_DELAY = 500;

/** Сколько держится ответ кнопки копирования. Столько же, сколько у паролей. */
const COPIED_FOR = 1500;

/**
 * Больше страниц не заводим.
 *
 * Точки стоят строкой над заметкой, и в узкое окно их помещается немного;
 * шесть заметок в виджете на обоях — это уже не пара мыслей под рукой, а
 * записная книжка, для которой есть приложения получше.
 */
const MAX_PAGES = 6;

/** Пустая книга: с ней карточка живёт, пока не пришёл ответ с диска. */
const NOTHING: Book = { pages: [], active: 0 };

/** Одна и та же книга. Сравниваем сами: писать ради того же самого незачем. */
function same(a: Book, b: Book): boolean {
  return (
    a.active === b.active &&
    a.pages.length === b.pages.length &&
    a.pages.every((page, at) => page === b.pages[at])
  );
}

export function NotesWidget() {
  useWindowDrag();
  const t = useText();
  const reference = useLook().effectiveDesign === "reference";
  const layout = useWidgetLayout();

  const [book, setBook] = useState<Book>(NOTHING);
  /**
   * Что не так с диском. Причина ушла в лог: в карточке ей места нет.
   *
   * Состояния два, и путать их нельзя: не прочиталось — заметки на диске целы,
   * а в карточке их нет; не записалось — в карточке они есть, а на диске лежат
   * прежние. Одно слово на оба случая соврало бы в одном из них.
   */
  const [alarm, setAlarm] = useState<"read" | "write" | null>(null);
  /** Ответ кнопки копирования: `undefined` — её ещё не нажимали. */
  const [copied, setCopied] = useState<boolean | undefined>(undefined);
  /**
   * Правка разрешена: заметку можно набирать.
   *
   * ВАЖНО: **по умолчанию поле заперто, и мышь до него не доходит вовсе.**
   * Заметка занимает всю карточку, и пока поле ловит мышь, одна и та же
   * протяжка в нём — это и выделение текста, и перенос окна; различать их по
   * времени или по фокусу пробовали, живой проверки это не прошло.
   *
   * Запертая заметка — обычное пустое место карточки: за неё тянут окно, по ней
   * открывается меню виджета, в ней нажимаются ссылки. Разрешает правку двойное
   * нажатие, запирает обратно щелчок мимо окна или Escape.
   */
  const [writing, setWriting] = useState(false);
  /**
   * Книга до стирания. Пока она тут, стирание можно отменить.
   *
   * Ctrl+Z в поле от этого не отменяется: он возвращает буквы, а кнопка —
   * страницу целиком, которой в откате поля нет вовсе. Разные глубины у одной
   * беды, и обе нужны.
   */
  const { kept: undo, remember, forget } = useUndo<Book>();

  /*
   * Поле нужно самой карточке: стирание идёт через него, а не через состояние.
   * Подробности — у `erase`.
   */
  const field = useRef<HTMLTextAreaElement>(null);
  /**
   * Подложка под полем: тот же текст теми же буквами, но со ссылками.
   *
   * ВАЖНО: **буквы рисует она, а поле стоит прозрачным.** Ссылку внутри
   * `textarea` не сделать нажимаемой никак — это одна сплошная строка без
   * разметки, — поэтому текст показывает обычный узел под ним, а поле остаётся
   * ради курсора, набора, выделения и отката. Метрики у них обязаны совпадать
   * до пикселя: один шрифт, один межстрочный, один перенос и одинаковая ширина
   * с местом под полосу прокрутки.
   */
  const ghost = useRef<HTMLDivElement>(null);

  /**
   * Что лежит на диске прямо сейчас.
   *
   * ВАЖНО: `null` означает «ещё не читали», и писать в этом состоянии нельзя.
   * Пустая карточка до прихода ответа — это не пустая книга, и запись из неё
   * стёрла бы написанное прошлым запуском.
   */
  const stored = useRef<Book | null>(null);
  /**
   * То же, что в `book`, но доступное из обещаний и обработчиков.
   *
   * Состояние там всегда прошлое — замкнуто на тот заход отрисовки, в котором
   * обещание завели, — а записывать надо то, что в карточке сейчас.
   */
  const latest = useRef<Book>(NOTHING);
  /** Книга последней начатой записи: по ней узнаётся отставший ответ. */
  const wanted = useRef<Book | null>(null);
  /** Ответы IPC могут прийти в другом порядке, чем снимки легли на диск. */
  const saving = useRef(0);
  const saved = useRef(0);

  /**
   * Кладёт книгу на диск, если она отличается от лежащей там.
   *
   * Ревизия в saveNotes сохраняет порядок снимков, а stored меняется только
   * после успеха: после двух отказов он не должен указывать на несохранённое.
   * wanted подавляет повтор уже ожидающего снимка; старый ответ не гасит
   * предупреждение о более новой попытке.
   */
  const keep = useCallback((next: Book) => {
    const before = stored.current;
    if (before === null || same(wanted.current ?? before, next)) return;

    wanted.current = next;
    const attempt = ++saving.current;

    saveNotes(next).then(
      () => {
        if (attempt > saved.current) {
          saved.current = attempt;
          stored.current = next;
        }
        if (saving.current === attempt) {
          wanted.current = null;
          setAlarm(null);
        }
      },
      () => {
        if (saving.current !== attempt) return;
        wanted.current = null;
        setAlarm("write");
      },
    );
  }, []);

  /**
   * Одна дверь на все правки книги: и набор, и страницы идут через неё.
   *
   * ВАЖНО: **любая правка снимает предложение вернуть.** Снимок хранит книгу
   * целиком, и возврат поверх написанного после стирания стёр бы его, а заодно
   * перебросил бы на страницу, открытую тогда.
   */
  const apply = useCallback(
    (next: Book, now = false) => {
      forget();
      latest.current = next;
      setBook(next);
      // Отложенная запись нужна набору, а не страницам, корзине и возврату:
      // выключенный следом модуль разбирает окно без уборки, и полсекунды
      // правки, сделанной не в поле, уходили вместе с ним — `onBlur` у поля
      // без фокуса не придёт
      if (now) keep(next);
    },
    [forget, keep],
  );

  useEffect(() => {
    let alive = true;

    loadNotes().then(
      (kept) => {
        if (!alive) return;
        stored.current = kept;
        latest.current = kept;
        setBook(kept);
      },
      () => {
        // Не прочиталось — не правим и не пишем вовсе: поверх нечитаемого
        // файла может лежать целая книга. `stored` остаётся пустым, и это и
        // есть запрет
        if (alive) setAlarm("read");
      },
    );

    return () => {
      alive = false;
    };
  }, []);

  /*
   * Ушли из окна — правка запирается снова. Возвращаться к заметке чаще всего
   * приходят читать, а не дописывать, и оставленное открытым поле означало бы,
   * что окно опять не тянется.
   */
  useEffect(() => {
    const lock = () => {
      // Уход из окна завершает набор, даже если blur самого поля не пришёл.
      keep(latest.current);
      setWriting(false);
    };

    window.addEventListener("blur", lock);
    return () => window.removeEventListener("blur", lock);
  }, [keep]);

  const settled = useDebounced(book, SAVE_DELAY);
  useEffect(() => {
    keep(settled);
  }, [settled, keep]);

  const answer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(answer.current), []);

  /*
   * Пустая книга показывается одной пустой страницей: ненаписанная заметка —
   * это страница, которую ещё не написали, а не отсутствие страниц. Сама
   * страница заводится при первой букве, а не при открытии окна: иначе виджет
   * писал бы файл, ничего не спросив у хозяина.
   */
  const text = book.pages[book.active] ?? "";
  const alone = book.pages.length <= 1;

  /*
   * ВАЖНО: **пока книга не прочитана, правок нет вовсе** — ни букв, ни страниц,
   * ни корзины. Пустая карточка до прихода ответа — это не пустая книга, а
   * любое действие в ней было бы действием над пустотой. Прежде правки до
   * прочтения пускались и считались важнее прочитанного — а правкой считалось и
   * нажатие по единственной точке страницы, что стоит у верхнего края, там, где
   * окно берут мышью. Дочитанная книга ему уступала, и на диск ложилась пустая:
   * так, судя по всему, 28 сентября и пропала живая заметка.
   *
   * Запрет стоит по ссылке `stored`, а не по состоянию: обработчики читают её в
   * миг нажатия. Не прочиталось вовсе — запрет остаётся до перезапуска: поверх
   * нечитаемого файла может лежать целая книга. Тем же путём давно живут дела.
   */
  const unread = () => stored.current === null;

  function edit(next: string, saveNow = false) {
    if (unread()) return;
    const now = latest.current;
    const pages =
      now.pages.length === 0
        ? [next]
        : now.pages.map((page, at) => (at === now.active ? next : page));

    apply({ pages, active: Math.min(now.active, pages.length - 1) }, saveNow);
  }

  function pick(at: number) {
    if (unread()) return;
    // Страницу переключают, чтобы посмотреть другую: правку при этом запираем,
    // иначе окно перестало бы тянуться за заметку, которую даже не трогали
    setWriting(false);
    apply({ ...latest.current, active: at }, true);
  }

  function add() {
    if (unread()) return;
    const now = latest.current;
    // Пустая книга уже показана одной страницей — её и заводим вместе с новой
    const pages = [...(now.pages.length === 0 ? [""] : now.pages), ""];
    if (pages.length > MAX_PAGES) return;

    apply({ pages, active: pages.length - 1 }, true);

    // Новую страницу заводят, чтобы писать: запертой она означала бы ещё одно
    // двойное нажатие сразу после нажатия «плюс»
    setWriting(true);
    window.setTimeout(() => field.current?.focus(), 0);
  }

  /**
   * Разрешает правку и ставит курсор туда, куда нажали.
   *
   * Место курсора берём у подложки: буквы рисует она, и попасть в них мышью
   * можно только по ней. Смещение считаем сложением длин её текстовых узлов до
   * того, в который попали, — разметка внутри плоская, глубже одного уровня в
   * ней ничего нет.
   *
   * Не вышло — курсор в конец. Это не беда: заметку правят с клавиатуры, а
   * промахнувшийся курсор человек поправит одним нажатием.
   */
  function unlock(event: ReactMouseEvent<HTMLDivElement>) {
    // Правку уже разрешили — двойное нажатие принадлежит полю: там оно выделяет
    // слово, и лезть в него со своим курсором значит это выделение стереть.
    // Не прочитанную с диска книгу не правят вовсе
    if (writing || unread()) return;

    setWriting(true);

    const box = field.current;
    const layer = ghost.current;
    if (box === null) return;

    let at = box.value.length;

    /*
     * ВАЖНО: **на время замера подложке возвращают мышь.** `caretRangeFromPoint`
     * считается с `pointer-events` и сквозь погашенный узел не смотрит вовсе —
     * без этого он отвечает коробкой, а не буквой, и курсор всегда вставал бы в
     * конец.
     */
    if (layer !== null) layer.style.pointerEvents = "auto";
    const spot = document.caretRangeFromPoint(event.clientX, event.clientY);
    if (layer !== null) layer.style.pointerEvents = "";

    if (spot !== null && layer !== null && layer.contains(spot.startContainer)) {
      let before = 0;
      const walk = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);

      while (walk.nextNode() !== null) {
        if (walk.currentNode === spot.startContainer) {
          at = before + spot.startOffset;
          break;
        }
        before += walk.currentNode.textContent?.length ?? 0;
      }
    }

    // Курсор ставим после того, как поле снова начнёт ловить мышь и фокус
    window.setTimeout(() => {
      box.focus();
      box.setSelectionRange(at, at);
    }, 0);
  }

  function copy() {
    void copyToClipboard(text).then((done) => {
      setCopied(done);
      window.clearTimeout(answer.current);
      answer.current = window.setTimeout(() => setCopied(undefined), COPIED_FOR);
    });
  }

  /*
   * Одна кнопка на два действия, и они идут по порядку: сперва стереть текст,
   * а у уже пустой страницы — убрать саму страницу. Так удаление заметки стоит
   * двух раздельных нажатий, и первое из них отменяется по Ctrl+Z.
   *
   * ВАЖНО: **стирание идёт правкой самого поля, а не заменой состояния.**
   * `execCommand` по выделенному попадает в откат поля, и Ctrl+Z возвращает
   * написанное. Заметку пишут руками, и нечаянное нажатие не должно уносить её
   * насовсем; подтверждение при этом не нужно — за него отвечает откат.
   * Состояние догоняет событием `input`, которое команда шлёт сама, а строкой
   * ниже мы это подтверждаем: пути отката может не оказаться.
   */
  function erase() {
    if (unread()) return;
    const box = field.current;
    const before = latest.current;

    if (text !== "") {
      /*
       * ВАЖНО: **через поле стираем только у разрешённой правки.** У запертого
       * поля стоит `readOnly`, а с ним команда правки не выполняется вовсе:
       * возвращает `false` и не меняет ни буквы. Замерено — до этого кнопка
       * корзины у запертой заметки не делала ничего, кроме кнопки возврата.
       *
       * Путь через поле нужен ради отката по Ctrl+Z, и у запертой заметки он
       * бесполезен: клавиатуры у неё всё равно нет. Там стираем состоянием, а
       * возвращает кнопка.
       */
      if (writing && box !== null) {
        box.focus();
        box.select();
        document.execCommand("delete");
      }

      edit("", true);
      // После самой правки, а не до: `apply` снимает предложение вернуть, и
      // снимок, положенный раньше, тут же был бы выброшен
      remember(before);
      return;
    }

    if (alone) return;

    const pages = before.pages.filter((_, at) => at !== before.active);
    apply({ pages, active: Math.min(before.active, pages.length - 1) }, true);
    remember(before);
  }

  function rollback() {
    if (undo === null) return;
    apply(undo, true);
  }

  return (
    <div className={reference ? "widget widget--notes-reference" : "widget"}>
      {/* Акцент нейтральный: заметка не железо, метрики у неё нет, а чужой
          цвет сказал бы о ней то, чего в ней нет */}
      <GlassCard accent="neutral">
        <div className={reference ? "notes notes--reference" : "notes"} data-layout={layout} data-writing={writing} data-alarm={alarm !== null}>
          <Dots
            scrollable={reference}
            count={book.pages.length}
            active={book.active}
            limit={MAX_PAGES}
            onPick={pick}
            onAdd={add}
            disabled={reference && unread()}
          />

          {/*
            Двойное нажатие разрешает правку. Ловит его коробка, а не поле:
            запертое поле мыши не видит вовсе.
          */}
          <div
            className="notes__box"
            data-writing={writing}
            /* Подсказка только у запертой: у разрешённой она врала бы */
            title={writing ? undefined : t("notes.editHint")}
            onDoubleClick={unlock}
            /* Запертое поле мыши не видит, и колесо прокручивало бы ничего:
               длинную заметку нельзя было дочитать без разрешения правки */
            onWheel={(event) => {
              if (writing || field.current === null) return;
              field.current.scrollTop += event.deltaY;
            }}
          >
            {/*
              Подложка с текстом и ссылками. Прокрутку ей задаёт поле: своей у
              неё нет, иначе рядом стояли бы две полосы.

              Лишний перевод строки в конце не украшение: у поля последняя
              пустая строка есть, а у обычного узла она схлопывается, и без него
              подложка уезжала бы вверх на строку.
            */}
            <div className="notes__ghost" ref={ghost} aria-hidden="true">
              <Linked text={text} live={!writing} verbatim />
              {"\n"}
            </div>

            <textarea
              ref={field}
              className="notes__field"
              value={text}
              placeholder={reference && unread() ? (alarm === "read" ? "" : t("common.loading")) : t("notes.placeholder")}
              /* Подчёркивания под каждым вторым словом на стекле читаются как
                 поломка, а язык заметки заранее неизвестен */
              spellCheck={false}
              /* Заперто — не только не правится, но и не ловит ни мышь, ни
                 клавиатуру: иначе в него можно было бы войти табуляцией и
                 набирать в обход двойного нажатия */
              readOnly={!writing}
              tabIndex={writing ? undefined : -1}
              onKeyDown={(event) => {
                // Escape запирает обратно: то же, что щелчок мимо окна
                if (event.key === "Escape") {
                  setWriting(false);
                  event.currentTarget.blur();
                }
              }}
              onChange={(event) => edit(event.target.value)}
              onScroll={() => {
                // Подложка едет за полем: её прокрутка целиком его
                if (ghost.current !== null && field.current !== null) {
                  ghost.current.scrollTop = field.current.scrollTop;
                }
              }}
              /* Уход из поля — конец мысли: ждать отложенной записи незачем.
                 Замок при этом не трогаем: нажатие по кнопке в подвале тоже
                 уводит фокус, а правку оно закрывать не должно */
              onBlur={() => keep(latest.current)}
            />
          </div>

          {/*
            Подвал: слева беда, справа значки. Одной строкой, как у буфера
            обмена, — вторая отняла бы у поля ещё одну.
          */}
          <div className="notes__foot">
            {/*
              Молчит, пока всё хорошо. Пустая заметка от жалобы не избавляет:
              стёртая и не записанная — это старый текст, который вернётся при
              следующем запуске, и промолчать тут значило бы соврать.
            */}
            <span className="notes__trouble" role={reference && alarm !== null ? "status" : undefined} title={alarm === "read" ? t("notes.cantRead") : alarm === "write" ? t("common.notSaved") : undefined}>
              {alarm === "read" ? t("notes.cantRead") : alarm === "write" ? t("common.notSaved") : ""}
            </span>

            {/*
              Кнопки без подложки — значками, как обновление у погоды. Рамка
              вокруг них была бы самой заметной вещью в карточке, где всё
              остальное набрано текстом.
            */}
            {/* Индикатор сообщает режим; привычный двойной щелчок остаётся. */}
            {reference && (
              <span className="notes__mode" title={writing ? t("notes.writing") : t("notes.editHint")} aria-label={writing ? t("notes.writing") : t("notes.editHint")}>
                {writing ? <PenIcon /> : <LockIcon />}
              </span>
            )}

            <button
              type="button"
              className="notes__act"
              title={copied === undefined ? t("notes.copy") : copied ? t("common.copied") : t("common.failed")}
              disabled={text === ""}
              onClick={copy}
            >
              {copied === undefined ? <CopyIcon /> : copied ? <DoneIcon /> : <FailedIcon />}
            </button>

            <button
              type="button"
              className="notes__act"
              title={text !== "" ? t("notes.erase") : t("pages.remove")}
              disabled={text === "" && alone}
              onClick={erase}
            >
              <EraseIcon />
            </button>
          </div>

          {/* Возврат стоит поверх содержимого, посреди карточки */}
          {undo !== null && <Undo onClick={rollback} />}
        </div>
      </GlassCard>
    </div>
  );
}
