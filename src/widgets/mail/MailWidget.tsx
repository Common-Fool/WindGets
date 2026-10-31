import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useText, type Key } from "../../lib/i18n";
import { LetterBrowser, LetterIncomplete } from "./LetterBrowser";
import { useLook } from "../../lib/look";
import { useWidgetLayout } from "../../lib/view";
import { ReferenceLetterPopupContent, ReferenceLetterRow } from "./MailReference";
import { fileSize, formatLetterTime } from "../../lib/format";
import { copyToClipboard } from "../../lib/copy";
import { openLink } from "../../lib/links";
import { openWidgetSettings } from "../../lib/settings";
import {
  byAccount,
  deleteLetter,
  getMailBody,
  letterKey,
  markAllMailRead,
  markMailRead,
  MAX_ACCOUNTS,
  refreshMail,
  searchMail,
  useMail,
  type Inbox,
  type Letter,
  type LetterBody,
  type MailBox,
} from "../../lib/mail";
import { GlassCard } from "../../components/GlassCard";
import { Dots } from "../../components/Dots";
import { Linked } from "../../components/Linked";
import {
  BoxCheckedIcon,
  BoxIcon,
  CheckAllIcon,
  CheckIcon,
  ClipIcon,
  CloseIcon,
  DotIcon,
  EraseIcon,
  ExternalLinkIcon,
  RefreshIcon,
} from "../../components/Icons";
import "./mail.css";
import "./mail-reference.css";

// Отказ «список устарел» узнаём по тексту: Rust берёт его из того же каталога
// и на том же языке (`MailError::Stale`), отдельного кода ошибки у команд нет
const STALE_LIST_KEY: Key = "mail.error.stale";

export type LetterRowProps = {
  /** Компоновка NEW: 0 — таблица, 1 — дайджест. */
  layout?: number;
  letter: Letter;
  onSelect: (letter: Letter) => void;
  /** Сделать и сказать, если не вышло. */
  act: (letter: Letter, work: Promise<void>, failed: string, kind: "read" | "delete") => void;
  /** Над письмом идёт работа: ответа с сервера ещё нет. */
  busy: boolean;
  /** Письмо отмечено. */
  picked: boolean;
  /** Отмечено хоть что-нибудь: тогда квадратики видны у всех строк. */
  anyPicked: boolean;
  onPick: (on: boolean) => void;
};

function LetterRow({
  letter,
  onSelect,
  act,
  busy,
  picked,
  anyPicked,
  onPick,
}: LetterRowProps) {
  const t = useText();
  return (
    <li
      className="mail__item"
      data-picked={picked}
      data-seen={letter.seen}
      onClick={() => onSelect(letter)}
    >
      <div className="mail__item-main">
        <div className="mail__item-head">
          {/*
            * Квадратик стоит на месте точки непрочитанного и занимает ровно
            * её место: они не бывают нужны одновременно, а два разных отступа
            * слева у соседних строк читались бы как сбитая раскладка.
            *
            * Виден он при наведении и пока хоть что-то отмечено — иначе,
            * отметив письмо, пришлось бы держать мышь на строке, чтобы видеть
            * отметку.
            */}
          <span className="mail__mark">
            <button
              type="button"
              className="mail__pick"
              data-on={picked}
              data-shown={anyPicked}
              title={picked ? t("mail.deselect") : t("mail.select")}
              aria-label={picked ? t("mail.deselect") : t("mail.select")}
              onClick={(event) => {
                event.stopPropagation();
                onPick(!picked);
              }}
              onDoubleClick={(event) => event.stopPropagation()}
            >
              {picked ? <BoxCheckedIcon /> : <BoxIcon />}
            </button>
            <span className="mail__dot" data-seen={letter.seen} data-hidden={anyPicked} />
          </span>
          <span className="mail__from">{letter.from}</span>
        </div>
        <div className="mail__subject">{letter.subject}</div>
        {/*
          * Строки содержимого может не быть вовсе: она стоит настройкой, и без
          * неё опрос не запрашивает тела писем совсем. Пустого места под неё не
          * держим — список прокручивается, и строки разной высоты ему не помеха.
          */}
        {letter.preview !== null && letter.preview !== "" && (
          <div className="mail__preview">{letter.preview}</div>
        )}
      </div>
      <div className="mail__item-side">
        <span className="mail__time tabular">{formatLetterTime(letter.at, t.locale)}</span>
        {/*
          * Пока идёт работа, значки уступают место кружку.
          *
          * Ждать тут есть чего: пометка открывает свою сессию IMAP, а следом
          * идёт полный опрос ящика — это секунды, и молчащая кнопка все эти
          * секунды выглядит нажатой впустую. Кружок держится до прихода
          * настоящего ответа, а не по таймеру.
          */}
        <div className="mail__item-actions" data-busy={busy}>
          {busy ? (
            <span className="spinner" title={t("mail.working")}>
              <RefreshIcon />
            </span>
          ) : (
            <>
              {!letter.seen && (
                <button
                  type="button"
                  className="mail__item-btn"
                  title={t("mail.markRead")}
                  aria-label={t("mail.markRead")}
                  onClick={(event) => {
                    event.stopPropagation();
                    act(letter, markMailRead(letter.account, [letter.uid], letter.revision, letter.uidValidity), "not marked", "read");
                  }}
                >
                  <CheckIcon />
                </button>
              )}
              <button
                type="button"
                className="mail__item-btn mail__item-btn--delete"
                title={t("mail.delete")}
                aria-label={t("mail.delete")}
                onClick={(event) => {
                  event.stopPropagation();
                  act(letter, deleteLetter(letter.account, [letter.uid], letter.revision, letter.uidValidity), "not deleted", "delete");
                }}
              >
                <EraseIcon />
              </button>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

/**
 * Строка адреса в открытом письме: нажали — адрес в буфере.
 *
 * Нажатием по самому адресу, а не значком рядом: адрес и есть то, что копируют,
 * и целиться в кнопку сбоку было бы лишним движением. Подтверждения нет
 * намеренно — нажатие видно и так, а подпись, на две секунды меняющая слово,
 * дёргала бы строку на ровном месте.
 */
function Address({ label, value }: { label: string; value: string }) {
  const t = useText();
  return (
    <div className="mail__popup-address">
      <span className="mail__popup-label">{label}</span>
      <button
        type="button"
        className="mail__popup-value"
        title={t("mail.copyAddress")}
        onClick={() => void copyToClipboard(value)}
      >
        {value}
      </button>
    </div>
  );
}

export type LetterPopupProps = {
  reference?: boolean;
  letter: Letter;
  onClose: () => void;
  /** Над письмом идёт работа: ответа с сервера ещё нет. */
  busy: boolean;
  onMarkRead: () => void;
};

function LetterPopup({ letter, onClose, busy, onMarkRead, reference = false }: LetterPopupProps) {
  const t = useText();
  /*
   * Текст письма просится отдельно и только здесь.
   *
   * Со списком он не едет вовсе: письма нынче сплошь HTML без текстовой части,
   * и в шестнадцать килобайт, которых списку хватает на превью, видимый текст
   * не попадает — до него доезжает одна шапка со стилями.
   */
  const [body, setBody] = useState<LetterBody | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState<"stale" | "body" | null>(null);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(null);
    setBody(null);

    getMailBody(letter)
      .then((loaded) => {
        if (!alive) return;
        setBody(loaded);
        setLoading(false);
      })
      .catch((error: unknown) => {
        if (!alive) return;
        setFailed(String(error) === t(STALE_LIST_KEY) ? "stale" : "body");
        setLoading(false);
      });

    return () => {
      alive = false;
    };
    // Тело прежнего поколения нельзя показывать письму с повторившимся UID.
  }, [letter.account, letter.revision, letter.uidValidity, letter.uid]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const failure = failed === null ? null : failed === "stale" ? t(STALE_LIST_KEY) : t("mail.popupFailed");
  if (reference) return <ReferenceLetterPopupContent letter={letter} onClose={onClose} busy={busy} onMarkRead={onMarkRead} body={body} loading={loading} failed={failure} />;

  return (
    <div className="mail__popup-overlay" onClick={onClose}>
      <div
        className="mail__popup"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="mail__popup-head">
          <div className="mail__popup-title-group">
            <h3 className="mail__popup-subject">{letter.subject}</h3>
            <div className="mail__popup-meta">
              <span className="mail__popup-from">{letter.from}</span>
              <span className="mail__popup-time tabular">{formatLetterTime(letter.at, t.locale)}</span>
            </div>
            {/*
              * Адреса строками под именем: имя отправителя врёт легко, адрес —
              * нет, и в письме от «вашего банка» смотрят именно на него.
              */}
            {letter.fromAddress !== null && (
              <Address label={t("mail.from")} value={letter.fromAddress} />
            )}
            {letter.to.length > 0 && <Address label={t("mail.to")} value={letter.to.join(", ")} />}
          </div>
          <div className="mail__popup-actions">
            {/*
              * Отметка прочтения стоит первой, а закрытие последним: закрытие
              * ищут на одном месте всегда, и подвинуть его появившейся слева
              * кнопкой значило бы промахиваться мимо него.
              *
              * У прочитанного письма отметка не прячется, а гаснет: пропавшая
              * кнопка сдвинула бы соседние, и вернувшись из письма в письмо,
              * человек нажимал бы не туда.
              */}
            {busy ? (
              <span className="mail__popup-btn spinner" title={t("mail.working")}>
                <RefreshIcon />
              </span>
            ) : (
              <button
                type="button"
                className="mail__popup-btn"
                title={letter.seen ? t("mail.alreadyRead") : t("mail.markRead")}
                aria-label={letter.seen ? t("mail.alreadyRead") : t("mail.markRead")}
                disabled={letter.seen}
                onClick={onMarkRead}
              >
                <CheckIcon />
              </button>
            )}
            {letter.link !== null && !body?.truncated && <LetterBrowser link={letter.link} className="mail__popup-btn" />}
            <button
              type="button"
              className="mail__popup-btn"
              title={t("settings.close")}
              aria-label={t("settings.close")}
              onClick={onClose}
            >
              <CloseIcon />
            </button>
          </div>
        </div>

        <div className="mail__popup-body" data-no-drag>
          {loading ? (
            <div className="mail__popup-empty">
              <span className="spinner">
                <RefreshIcon />
              </span>
            </div>
          ) : failed ? (
            <div className="mail__popup-empty">{failure}</div>
          ) : body !== null && body.text !== null && body.text.trim() !== "" ? (
            <div className="mail__popup-text">
              {/*
                * Ссылки в письме нажимаются — тем же `Linked`, что в заметках и
                * делах. Настоящего `<a href>` тут нет: переход увёл бы само
                * окно виджета на страницу, а вернуться из него нечем.
                */}
              <Linked text={body.text} />
            </div>
          ) : (
            <div className="mail__popup-empty">{t("mail.popupEmpty")}</div>
          )}

          {/*
            * Вложения внизу письма: имя, тип и размер. Содержимого их тут нет
            * и не будет — виджет письма показывает, а не хранит.
            */}
          {body !== null && body.attachments.length > 0 && (
            <ul className="mail__files">
              {body.attachments.map((file, at) => (
                <li className="mail__file" key={`${file.name}-${at}`}>
                  <span className="mail__file-icon">
                    <ClipIcon />
                  </span>
                  <span className="mail__file-name">{file.name}</span>
                  <span className="mail__file-size tabular">{fileSize(file.size)}</span>
                </li>
              ))}
            </ul>
          )}
          {body?.truncated && <LetterIncomplete link={letter.link} />}
        </div>
      </div>
    </div>
  );
}

/** Свежее сверху; одно время у двух писем — старший UID выше, как в ящике. */
function newestFirst(letters: Letter[]): Letter[] {
  return [...letters].sort((a, b) => b.at - a.at || b.uid - a.uid);
}

/** Входящие ящика, если снимок есть. */
function inboxOf(box: MailBox): { inbox: Inbox; stale: boolean } | null {
  return box.view.status === "ready" ? { inbox: box.view.inbox, stale: box.view.stale } : null;
}

/**
 * Кнопка во всю карточку, ведущая в настройки: ящиков нет, ящик заведён не до
 * конца или сервер не пустил. Подпись, зовущая в настройки, но никуда не
 * ведущая, была бы издевательством.
 */
function SetupButton({ page, title, hint, error = false }: { page: string; title: string; hint: string; error?: boolean }) {
  return (
    <button type="button" className="mail__setup-btn" data-drags onClick={() => openWidgetSettings("mail", page)}>
      <span className={error ? "mail__status-title mail__status-title--error" : "mail__status-title"}>{title}</span>
      <span className="mail__status-hint">{hint}</span>
    </button>
  );
}

export function MailWidget() {
  const t = useText();
  const reference = useLook().effectiveDesign === "reference";
  const layout = useWidgetLayout() === 1 ? 1 : 0;

  const mail = useMail();
  const accounts = mail?.accounts ?? [];
  /*
   * Вкладка: `null` — «Все», иначе ключ ящика.
   *
   * Живёт до закрытия окна, как переключатель ЦПУ/ОЗУ у процессов: файла
   * такое не заслуживает. Открывается на «Все» — ради неё вкладки и заведены.
   * Удалённый ящик возвращает на «Все» сам: его ключа в снимке больше нет.
   */
  const [tab, setTab] = useState<string | null>(null);
  const current = tab !== null && accounts.some((box) => box.id === tab) ? tab : null;
  // Вкладки нужны от двух ящиков: у одного «Все» и он сам — одно и то же
  const tabbed = accounts.length > 1;
  const inTab = current === null ? accounts : accounts.filter((box) => box.id === current);
  const lone = inTab.length === 1 ? inTab[0]! : null;

  const [spinning, setSpinning] = useState(false);
  const [selectedLetter, setSelectedLetter] = useState<Letter | null>(null);
  /*
   * Осечка действия: не пометилось, не удалилось.
   *
   * Показать её обязательно — молча проглоченный отказ выглядит как «кнопка не
   * работает», а причины у него бывают настоящие: у сервера может не найтись
   * корзины, и тогда письмо намеренно остаётся на месте.
   */
  const [trouble, setTrouble] = useState<string | null>(null);
  /*
   * Письма, над которыми идёт работа, — ключами letterKey с поколением папки.
   *
   * Список, а не одно письмо: пока идёт пометка одного, ничто не мешает нажать
   * по второму, и второй кружок обязан появиться там же, где нажали.
   */
  const [busy, setBusy] = useState<string[]>([]);
  const [markingAll, setMarkingAll] = useState(false);
  /*
   * Полоса прокрутки видна, только пока крутят.
   *
   * Стоящая всегда, она рядом со строкой письма читается как ещё один столбец
   * содержимого. Гаснет через паузу после последнего движения — не по нажатию:
   * колесо мыши шлёт события пачками, и полоса моргала бы между ними.
   */
  const [scrolling, setScrolling] = useState(false);
  const scrollTimer = useRef<number | undefined>(undefined);
  /*
   * Поиск живёт только в карточке: состояние виджета он не подменяет, и
   * фоновый опрос идёт своим чередом. Закрыли поиск — вернулся обычный список,
   * уже обновившийся.
   */
  const [query, setQuery] = useState("");
  const [unseenOnly, setUnseenOnly] = useState(false);
  const [found, setFound] = useState<Inbox | null>(null);
  const [looking, setLooking] = useState(false);
  /** Номер последнего поиска: ответ опоздавшего, со сменой вкладки, не нужен. */
  const asked = useRef(0);
  /*
   * Отмеченные письма — ключами letterKey, а не строками.
   *
   * Список под ними живой: пока их отмечали, мог прийти опрос и подвинуть
   * порядок. Ключ письма переживает и опрос, и переход к поиску.
   */
  const [picked, setPicked] = useState<string[]>([]);
  const accountVersion = accounts.map((box) => `${box.id}:${box.revision}:${box.view.status === "ready" ? box.view.inbox.uidValidity ?? "none" : "waiting"}`).join("|");
  const liveVersion = useRef(accountVersion);
  liveVersion.current = accountVersion;
  const repeatSearch = useRef(() => {});
  repeatSearch.current = () => {
    if (query.trim() !== "" || unseenOnly) look(query, unseenOnly);
  };

  // Новые реквизиты или UIDVALIDITY отменяют выбор, окно и опоздавшие ответы.
  useLayoutEffect(() => {
    asked.current++;
    setFound(null);
    setLooking(false);
    setSelectedLetter(null);
    setPicked([]);
    setBusy([]);
    repeatSearch.current();
  }, [accountVersion]);

  useEffect(() => () => window.clearTimeout(scrollTimer.current), []);

  function onScroll() {
    setScrolling(true);
    window.clearTimeout(scrollTimer.current);
    scrollTimer.current = window.setTimeout(() => setScrolling(false), 900);
  }

  /*
   * Значок крутится, пока не придёт ответ, и останавливается только на нём.
   *
   * Своего срока у вращения нет намеренно: значок, остановившийся сам по себе,
   * обещал бы свежую почту, которой нет. Ответ приходит всегда — на нажатие
   * кнопки Rust шлёт состояние даже тогда, когда оно не изменилось.
   */
  // Занятость писем этот снимок не снимает: он мог прийти от опроса другого
  // ящика, пока удаление ещё ждёт сервер, — и кнопка, вернувшаяся раньше
  // времени, пускала второе удаление (на сервере без MOVE — дубль в корзине).
  // Её снимает конец самого действия, см. `act` и `bulk`
  useEffect(() => {
    setSpinning(false);
    setMarkingAll(false);
  }, [mail]);

  useEffect(() => {
    if (trouble === null) return;
    const timer = window.setTimeout(() => setTrouble(null), 5000);
    return () => window.clearTimeout(timer);
  }, [trouble]);

  function refresh() {
    setSpinning(true);
    void refreshMail();
  }

  /*
   * Ищем по нажатию Enter, а не на каждую букву.
   *
   * `SEARCH TEXT` по большому ящику сервер считает секундами, и поиск на
   * каждое нажатие клавиши означал бы очередь запросов, каждый из которых уже
   * никому не нужен.
   *
   * На вкладке «Все» спрашиваем каждый ящик и сливаем ответы: один ящик, не
   * ответивший вовремя, не отменяет найденного в остальных.
   */
  function look(text: string, unread: boolean, boxes: MailBox[] = inTab) {
    const phrase = text.trim();
    const number = ++asked.current;
    const version = accountVersion;
    if (phrase === "" && !unread) {
      setFound(null);
      setLooking(false);
      return;
    }

    const ids = boxes.filter((box) => box.view.status !== "noAccount").map((box) => box.id);
    setLooking(true);
    setTrouble(null);
    setPicked([]);
    void Promise.allSettled(ids.map((id) => searchMail(id, phrase, unread))).then((answers) => {
      if (number !== asked.current || version !== liveVersion.current) return;
      const inboxes = answers.flatMap((answer) => (answer.status === "fulfilled" ? [answer.value] : []));
      setLooking(false);
      if (inboxes.length < answers.length) setTrouble("search failed");
      if (inboxes.length === 0 && answers.length > 0) {
        setFound(null);
        return;
      }

      // Поиск тоже может первым увидеть новое поколение, раньше фонового опроса.
      setSelectedLetter((selected) => {
        if (selected === null) return null;
        const at = ids.indexOf(selected.account);
        const answer = answers[at];
        return answer?.status === "fulfilled" && answer.value.uidValidity !== selected.uidValidity ? null : selected;
      });
      setFound({
        // Слитый список не имеет общего поколения: оно есть у каждого письма.
        uidValidity: inboxes.length === 1 ? inboxes[0]!.uidValidity : null,
        unseen: inboxes.reduce((sum, inbox) => sum + inbox.unseen, 0),
        letters: newestFirst(inboxes.flatMap((inbox) => inbox.letters)),
        mailbox: inboxes.length === 1 ? inboxes[0]!.mailbox : null,
      });
    });
  }

  function clearSearch() {
    setQuery("");
    if (!unseenOnly) {
      asked.current++;
      setFound(null);
      setLooking(false);
      return;
    }
    // Фильтр остаётся включённым: сняли слово, а не фильтр
    look("", true);
  }

  /**
   * Сменить вкладку: отметки снимаются, поиск переспрашивается.
   *
   * Найденное прежней вкладкой уходит сразу, не дожидаясь нового ответа: иначе
   * до него — а при отказе и насовсем — на вкладке одного ящика стояли бы
   * письма другого, и «отметить все» с «удалить» ушли бы в чужой ящик.
   */
  function pick(next: string | null) {
    setTab(next);
    setPicked([]);
    setSelectedLetter(null);
    setFound(null);
    if (query.trim() !== "" || unseenOnly) {
      look(query, unseenOnly, next === null ? accounts : accounts.filter((box) => box.id === next));
    }
  }

  /*
   * Что показать сразу после удачного действия.
   *
   * ВАЖНО: **сам по себе список найденного не обновится никогда.** Опрос
   * ящика, который Rust заводит после каждого действия, приносит новые
   * входящие — а на экране в это время может стоять результат поиска, и он
   * остаётся прежним. Выглядит это так, будто кнопка не сработала: письма
   * удалены на сервере, а в карточке их сколько было, столько и есть.
   *
   * Поэтому сперва правим показанное сами — раз обещание сдержано, дело на
   * сервере сделано, — а следом переспрашиваем сервер, чтобы разойтись с ним
   * было негде.
   */
  function settled(keys: string[], kind: "read" | "delete") {
    if (accountVersion !== liveVersion.current) return;
    setFound((was) => {
      if (was === null) return was;
      const letters =
        kind === "delete"
          ? was.letters.filter((letter) => !keys.includes(letterKey(letter)))
          : was.letters.map((letter) =>
              keys.includes(letterKey(letter)) ? { ...letter, seen: true } : letter,
            );
      return {
        ...was,
        letters,
        unseen: letters.filter((letter) => !letter.seen).length,
      };
    });

    repeatSearch.current();
  }

  /**
   * Действие над отмеченными: по одной сессии на ящик, снять отметки, сказать
   * об осечке. Отмечают по ключу, а письма берутся из того списка, что на
   * экране: там их и отмечали.
   */
  function bulk(kind: "read" | "delete", failed: string) {
    const chosen = shown.filter((letter) => picked.includes(letterKey(letter)));
    const keys = chosen.map(letterKey);
    const groups = [...byAccount(chosen)];
    setTrouble(null);
    setBusy((was) => [...new Set([...was, ...keys])]);
    setPicked([]);
    void Promise.allSettled(groups.map(([account, uids]) => {
      const source = chosen.find((letter) => letter.account === account)!;
      return kind === "read" ? markMailRead(account, uids, source.revision, source.uidValidity) : deleteLetter(account, uids, source.revision, source.uidValidity);
    })).then((answers) => {
      if (accountVersion !== liveVersion.current) return;
      const succeeded = groups.filter((_, at) => answers[at]!.status === "fulfilled").map(([account]) => account);
      const done = chosen.filter((letter) => succeeded.includes(letter.account)).map(letterKey);
      if (done.length > 0) settled(done, kind);
      const refused = answers.find((answer) => answer.status === "rejected" && String(answer.reason) === t(STALE_LIST_KEY));
      if (answers.some((answer) => answer.status === "rejected")) setTrouble(refused ? "stale" : failed);
      setBusy((was) => was.filter((key) => !keys.includes(key)));
    });
  }

  /**
   * Сделать и сказать, если не вышло.
   *
   * Кружок гасит конец самого действия: к этому мигу `settled` уже поправил
   * показанное, и письмо не выглядит по-старому до следующего опроса.
   */
  function act(letter: Letter, work: Promise<void>, failed: string, kind: "read" | "delete") {
    const key = letterKey(letter);
    setTrouble(null);
    setBusy((was) => (was.includes(key) ? was : [...was, key]));
    work
      .then(() => settled([key], kind))
      .catch((error: unknown) => {
        if (accountVersion !== liveVersion.current) return;
        setTrouble(String(error) === t(STALE_LIST_KEY) ? "stale" : failed);
      })
      .finally(() => setBusy((was) => was.filter((one) => one !== key)));
  }

  const inboxes = inTab.flatMap((box) => {
    const ready = inboxOf(box);
    return ready === null ? [] : [ready];
  });
  const unseen = inboxes.reduce((sum, ready) => sum + ready.inbox.unseen, 0);
  const letters = newestFirst(inboxes.flatMap((ready) => ready.inbox.letters));
  // Веб-ящик один — у одного ящика; у «Все» открывать было бы нечего одного
  const mailbox = lone !== null && inboxes.length === 1 ? inboxes[0]!.inbox.mailbox : null;
  // Снимок с прошлого удачного захода: писем столько же, но они уже не сегодняшние
  const stale = inboxes.some((ready) => ready.stale);
  const loading = inTab.some((box) => box.view.status === "loading");

  /*
   * Что показывает список: найденное, пока поиск открыт и что-то нашлось, —
   * иначе обычные входящие.
   */
  // Отбор — то, что правда искали, а не текст в поле: пока его набирают до
  // Enter, список не пропадает, а стёртый запрос возвращает входящие, а не
  // прежнее найденное
  const filtering = unseenOnly || (query.trim() !== "" && (found !== null || looking));
  const shown = (filtering ? (found?.letters ?? []) : letters).filter((letter) =>
    inTab.some((box) => box.id === letter.account && box.revision === letter.revision),
  );

  /*
   * Письмо для всплывающего окна берётся из того списка, что сейчас на экране,
   * и только потом из входящих.
   *
   * ВАЖНО: **искать его в одних входящих нельзя.** Найденное поиском письмо
   * старше последнего десятка среди них не лежит вовсе, и нажатие по такой
   * строке не открывало ровно ничего — окно закрывалось в тот же кадр, в
   * котором открылось. Второй список нужен затем, чтобы закрытый поиск не
   * захлопывал уже открытое письмо.
   *
   * Пропажа из обоих списков закрывает окно, и это то самое поведение, ради
   * которого письмо ищут каждый раз заново: удалённое письмо читать не из чего.
   */
  const selectedKey = selectedLetter === null ? null : letterKey(selectedLetter);
  const currentLetter =
    selectedKey !== null
      ? shown.find((item) => letterKey(item) === selectedKey) ??
        letters.find((item) => letterKey(item) === selectedKey) ??
        null
      : null;
  const allPicked = shown.length > 0 && shown.every((letter) => picked.includes(letterKey(letter)));

  /*
   * Ящики, которые на «Все» не отвечают: каждый своей строкой с адресом.
   * Молчащий ящик не гасит остальные — их письма стоят в списке, — но и
   * пропасть молча он не должен.
   */
  const troubles = lone === null ? inTab.filter((box) => box.view.status === "failed" || box.view.status === "noAccount") : [];

  const referenceTrouble = trouble === "not marked" ? t("mail.notMarked") : trouble === "not deleted" ? t("mail.notDeleted") : trouble === "search failed" ? t("mail.searchFailed") : trouble === "stale" ? t(STALE_LIST_KEY) : null;
  const referenceCorner = picked.length > 0 ? t.count("mail.selectedCount", picked.length) : [unseen > 0 ? t.count("mail.unreadCount", unseen) : null, stale ? t("mail.offline") : null, referenceTrouble].filter((piece) => piece !== null).join(" · ");

  let body: ReactNode;
  if (mail !== null && accounts.length === 0) {
    body = <SetupButton page="new" title={t("mail.noAccount")} hint={t("mail.noAccountHint")} />;
  } else if (lone !== null && lone.view.status === "noAccount") {
    body = <SetupButton page={lone.id} title={t("mail.noAccount")} hint={t("mail.noAccountHint")} />;
  } else {
    body = (
      <>
        {/* Поиск забирает всё место, оставшееся между ящиками, счётом
            и действиями. Все инструменты доступны и во время ввода. */}
        <div className="mail__head" onMouseDown={(event) => {
          // На ручном минимуме горизонтальный бегунок прокручивает шапку,
          // а её свободное место по-прежнему служит ручкой окна.
          const head = event.currentTarget;
          const scrollbar = head.offsetHeight - head.clientHeight;
          if (scrollbar > 0 && event.clientY >= head.getBoundingClientRect().top + head.clientTop + head.clientHeight) {
            event.stopPropagation();
          }
        }}>
          {tabbed && (
            <div className="mail__tabs">
              <Dots count={accounts.length + 1}
                active={current === null ? 0 : accounts.findIndex((box) => box.id === current) + 1}
                limit={MAX_ACCOUNTS + 1}
                titles={[t("mail.allBoxes"), ...accounts.map((box) => box.address)]}
                onPick={(at) => pick(at === 0 ? null : accounts[at - 1]!.id)} />
            </div>
          )}
          <div className="mail__search-box">
            <input
              type="text"
              className="input mail__search"
              value={query}
              placeholder={t("mail.search")}
              aria-label={t("mail.search")}
              title={query || t("mail.search")}
              spellCheck={false}
              autoComplete="off"
              data-no-drag
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  look(query, unseenOnly);
                  event.currentTarget.blur();
                }
                if (event.key === "Escape") clearSearch();
              }}
            />
            {query !== "" && (
              <button type="button" className="mail__head-btn"
                title={t("mail.clearSearch")} aria-label={t("mail.clearSearch")} onClick={clearSearch}>
                <CloseIcon />
              </button>
            )}
          </div>
          {referenceCorner !== "" && (
            <span className="mail__unread" data-error={trouble !== null} title={referenceCorner}>
              {referenceCorner}
            </span>
          )}
          <div className="mail__controls">
            {filtering && shown.length > 0 && (
              <button type="button" className="mail__pick mail__pick--all"
                data-on={allPicked} data-shown={true}
                title={allPicked ? t("mail.deselectAll") : t("mail.selectAll")}
                aria-label={allPicked ? t("mail.deselectAll") : t("mail.selectAll")}
                onClick={() => setPicked(allPicked ? [] : shown.map(letterKey))}>
                {allPicked ? <BoxCheckedIcon /> : <BoxIcon />}
              </button>
            )}
            {picked.length > 0 ? (
              <>
                <button type="button" className="mail__head-btn"
                  title={t("mail.markRead")} aria-label={t("mail.markRead")} onClick={() => bulk("read", "not marked")}>
                  <CheckIcon />
                </button>
                <button type="button" className="mail__head-btn mail__head-btn--danger"
                  title={t("mail.delete")} aria-label={t("mail.delete")} onClick={() => bulk("delete", "not deleted")}>
                  <EraseIcon />
                </button>
                <button type="button" className="mail__head-btn"
                  title={t("mail.clearSelection")} aria-label={t("mail.clearSelection")} onClick={() => setPicked([])}>
                  <CloseIcon />
                </button>
              </>
            ) : (
              <>
                <button type="button" className="mail__head-btn" data-on={unseenOnly}
                  title={unseenOnly ? t("mail.showAll") : t("mail.unreadOnly")}
                  aria-label={unseenOnly ? t("mail.showAll") : t("mail.unreadOnly")}
                  onClick={() => {
                    const next = !unseenOnly;
                    setUnseenOnly(next);
                    look(query, next);
                  }}>
                  <DotIcon />
                </button>
                {unseen > 0 && (markingAll ? (
                  <span className="spinner" title={t("mail.working")}><RefreshIcon /></span>
                ) : (
                  <button type="button" className="mail__head-btn"
                    title={t("mail.markAllRead")} aria-label={t("mail.markAllRead")}
                    onClick={() => {
                      const targets = inTab.filter((box) => (inboxOf(box)?.inbox.unseen ?? 0) > 0);
                      setTrouble(null);
                      setMarkingAll(true);
                      void Promise.allSettled(targets.map(markAllMailRead)).then((answers) => {
                        if (accountVersion !== liveVersion.current) return;
                        const succeeded = targets.filter((_, at) => answers[at]!.status === "fulfilled").map((box) => box.id);
                        const done = shown.filter((letter) => succeeded.includes(letter.account)).map(letterKey);
                        if (done.length > 0) settled(done, "read");
                        if (answers.some((answer) => answer.status === "rejected")) setTrouble("not marked");
                        setMarkingAll(false);
                      });
                    }}>
                    <CheckAllIcon />
                  </button>
                ))}
                {mailbox !== null && (
                  <button type="button" className="mail__head-btn"
                    title={t("mail.openMailbox")} aria-label={t("mail.openMailbox")} onClick={() => openLink(mailbox)}>
                    <ExternalLinkIcon />
                  </button>
                )}
                {looking ? <span className="spinner" title={t("mail.searching")}><RefreshIcon /></span> : (
                  <button type="button" className="refresh" data-spinning={spinning}
                    title={t("mail.refresh")} aria-label={t("mail.refresh")} onClick={refresh}>
                    <RefreshIcon />
                  </button>
                )}
              </>
            )}
          </div>
        </div>
        {reference && referenceTrouble !== null && <div className="mail-ref__trouble" role="status">{referenceTrouble}</div>}

        {/*
          * Ящики, которые на «Все» не отвечают или заведены не до конца, —
          * строкой каждый. Отказ входа и незаведённый ящик ведут на его
          * страницу настроек: чинятся они ровно там.
          */}
        {troubles.map((box) => {
          const auth = box.view.status === "noAccount" || (box.view.status === "failed" && box.view.kind === "auth");
          const label = `${box.address} · ${t(
            box.view.status === "noAccount" ? "mail.noAccount" : auth ? "mail.signInFailed" : "mail.unavailable",
          )}`;
          return auth ? (
            <button
              key={box.id}
              type="button"
              className="mail__box-trouble"
              title={t("mail.signInHint")}
              onClick={() => openWidgetSettings("mail", box.id)}
            >
              {label}
            </button>
          ) : (
            <div key={box.id} className="mail__box-trouble">
              {label}
            </div>
          );
        })}

        {(mail === null || (loading && inboxes.length === 0)) && (
          <div className="mail__status">
            <span className="mail__status-title">{t("common.loading")}</span>
          </div>
        )}

        {/*
          * Два отказа, и лечатся они по-разному: сервер не пустил —
          * менять пароль, не достучались — ждать. Русский `reason` рядом
          * с ними в карточку не идёт, он для лога.
          */}
        {lone !== null &&
          lone.view.status === "failed" &&
          (lone.view.kind === "auth" ? (
            /*
             * Отказ входа чинится ровно в одном месте, и карточка ведёт
             * туда сама — на страницу этого ящика.
             */
            <SetupButton page={lone.id} title={t("mail.signInFailed")} hint={t("mail.signInHint")} error />
          ) : (
            <div className="mail__status mail__status--error">
              <span className="mail__status-title">{t("mail.unavailable")}</span>
            </div>
          ))}

        {/*
          * Пока поиск открыт, список показывает найденное. Ничего не
          * нашлось — так и говорим: пустой список тут неотличим от
          * пустого ящика.
          */}
        {filtering && !looking && found !== null && found.letters.length === 0 && (
          <div className="mail__status">
            <span className="mail__status-title">{t("mail.searchNothing")}</span>
          </div>
        )}

        {!filtering && inboxes.length > 0 && !loading && letters.length === 0 && (
          <div className="mail__status">
            <span className="mail__status-title">{t("mail.none")}</span>
          </div>
        )}

        {shown.length > 0 && (
          /*
           * Список прокручивается, а не режется по высоте окна: письма
           * приходят десятком, а в карточку их влезает четыре-пять, и
           * остальные были бы просто недоступны.
           */
          <ul className="mail__list" data-scrolling={scrolling} onScroll={onScroll}>
            {shown.map((letter) => {
              const key = letterKey(letter);
              const Row = reference ? ReferenceLetterRow : LetterRow;
              return (
                <Row
                  layout={layout}
                  key={key}
                  letter={letter}
                  onSelect={setSelectedLetter}
                  act={act}
                  busy={busy.includes(key)}
                  picked={picked.includes(key)}
                  anyPicked={picked.length > 0}
                  onPick={(on) => setPicked((was) => (on ? [...was, key] : was.filter((one) => one !== key)))}
                />
              );
            })}
          </ul>
        )}

        {currentLetter !== null && (
          <LetterPopup
            reference={reference}
            letter={currentLetter}
            onClose={() => setSelectedLetter(null)}
            busy={busy.includes(letterKey(currentLetter))}
            onMarkRead={() =>
              act(
                currentLetter,
                markMailRead(currentLetter.account, [currentLetter.uid], currentLetter.revision, currentLetter.uidValidity),
                "not marked",
                "read",
              )
            }
          />
        )}
      </>
    );
  }

  return (
    <div className={reference ? "widget widget--mail-reference" : "widget"}>
      <GlassCard accent="neutral">
        <div className={reference ? "mail mail--reference" : "mail"} data-layout={reference ? layout : undefined}>
          {/*
            * Вкладки — точки, как страницы у заметок и дел: первая «Все», дальше
            * по ящику, адрес — подсказкой. Стоят над всем остальным и видны при
            * любом состоянии ящика: иначе с вкладки незаведённого ящика было бы
            * не уйти. «+» тут нет — ящики заводят в настройках.
            */}
          {tabbed && lone?.view.status === "noAccount" && (
            <Dots
              count={accounts.length + 1}
              active={current === null ? 0 : accounts.findIndex((box) => box.id === current) + 1}
              limit={MAX_ACCOUNTS + 1}
              titles={[t("mail.allBoxes"), ...accounts.map((box) => box.address)]}
              onPick={(at) => pick(at === 0 ? null : accounts[at - 1]!.id)}
            />
          )}
          {body}
        </div>
      </GlassCard>
    </div>
  );
}
