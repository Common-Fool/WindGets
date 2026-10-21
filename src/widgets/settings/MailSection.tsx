import { useEffect, useRef, useState } from "react";
import { Button } from "../../components/Button";
import { Check } from "../../components/Check";
import {
  checkMail,
  forgetMailAccount,
  getMailAccount,
  MAX_ACCOUNTS,
  setMailAccount,
  setMailPreview,
  useMail,
  type CheckStage,
  type MailAccountConfig,
  type MailBox,
  type MailCheck,
} from "../../lib/mail";
import { useText, type Key } from "../../lib/i18n";
import { useNavigate } from "./nav";

/*
 * Почта в окне настроек.
 *
 * Ящиков бывает несколько (этап 53, слово пользователя), и у каждого своя
 * страница — листом в дереве под «Почтой», как страницы виджетов под
 * «Виджетами». Страница самого виджета держит вид карточки и список ящиков: у
 * выключенного модуля дерева нет, и иначе до ящиков было бы не дойти.
 */

/** Чем ящик сейчас занят — строкой под адресом в списке. */
function stateOf(box: MailBox): Key | null {
  switch (box.view.status) {
    case "noAccount":
      return "mail.noAccount";
    case "loading":
      return "common.loading";
    case "failed":
      return box.view.kind === "auth" ? "mail.signInFailed" : "mail.unavailable";
    case "ready":
      return null;
  }
}

/**
 * Ящики — первым блоком на странице почты: по строке на ящик и «Добавить
 * ящик». Строка ведёт на страницу ящика.
 */
export function MailBoxesSection() {
  const t = useText();
  const go = useNavigate();
  const mail = useMail();
  const boxes = mail?.accounts ?? [];

  return (
    <div className="rows">
      <div className="field">
        <div className="field__head" data-hint={t("mail.settings.boxesHint")}>
          <span className="field__name">{t("mail.settings.boxes")}</span>
          <span className="field__value tabular">
            {boxes.length} / {MAX_ACCOUNTS}
          </span>
        </div>

        {boxes.length === 0 && <p className="note">{t("mail.settings.noBoxes")}</p>}

        {boxes.length > 0 && (
          <div className="mail-boxes">
            {boxes.map((box) => {
              const state = stateOf(box);
              return (
                <button key={box.id} type="button" className="mail-boxes__row" title={box.address} onClick={() => go(`mail/${box.id}`)}>
                  <span className="mail-boxes__address">{box.address || t("mail.settings.newBox")}</span>
                  {state !== null && <span className="mail-boxes__state">{t(state)}</span>}
                </button>
              );
            })}
          </div>
        )}

        <div className="choices">
          <Button variant="outline" disabled={boxes.length >= MAX_ACCOUNTS} onClick={() => go("mail/new")}>
            {t("mail.settings.add")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * Последняя проверка каждого ящика — до закрытия приложения.
 *
 * Новый ящик, сохранённый и проверенный, переходит на свою страницу, и форма
 * собирается заново под его ключом: без этой памяти результат проверки
 * пропадал бы ровно в тот миг, когда его хотят прочитать.
 */
const lastChecks = new Map<string, MailCheck>();

/**
 * Один ящик: форма его учётной записи. `id` — ключ ящика или `new`.
 *
 * Пароль бэкенд не возвращает никогда, поэтому поле пароля всегда пустое:
 * пустая строка при сохранении означает «оставить прежний пароль».
 *
 * Сохранение сразу проверяет подключение, а рядом есть своя кнопка проверки
 * (этап 54, слово пользователя: «не ясно, почему не подключило почту»). Ответ —
 * на каком шаге встало и что сказал сервер, прямо под кнопками.
 *
 * Строки содержимого тут нет: это вид карточки, одна на все ящики, и она стоит
 * на странице самой почты (`MailPreviewField`). Сохранение формы её не трогает.
 */
export function MailAccountSection({ id }: { id: string }) {
  // Другая страница — другой черновик, включая пароль и сохранённую проверку.
  return <MailAccountForm key={id} id={id} />;
}

function MailAccountForm({ id }: { id: string }) {
  const t = useText();
  const go = useNavigate();
  const fresh = id === "new";
  const [check, setCheck] = useState<MailCheck | null>(() => lastChecks.get(id) ?? null);
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [host, setHost] = useState("");
  const [port, setPort] = useState<string | number>(993);
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [mailbox, setMailbox] = useState("");
  const [mailboxHint, setMailboxHint] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    let alive = true;
    if (!fresh) getMailAccount(id)
      // Поля доступны до ответа: то, что человек успел вписать, ответ не
      // затирает — подставляется только в нетронутое
      .then((account) => {
        if (!alive) return;
        setHost((was) => was || account.host || "");
        setPort((was) => (was === 993 ? account.port || 993 : was));
        setLogin((was) => was || account.login || "");
        setHasPassword(Boolean(account.hasPassword));
        setMailbox((was) => was || account.mailbox || "");
        setMailboxHint(account.mailboxHint || "");
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
      active.current = false;
    };
  }, [id, fresh]);

  const form = (): MailAccountConfig => ({
    host: host.trim(),
    port: Number(port) || 993,
    login: login.trim(),
    mailbox: mailbox.trim(),
  });
  const typed = (): string | null => (password === "" ? null : password);

  /**
   * Проверить подключение и запомнить ответ. `key` — чей пароль брать, если
   * поле пустое; у нового ящика его нет, и тогда пароль обязан быть введён.
   */
  async function probe(key: string | null, pwd: string | null): Promise<void> {
    setChecking(true);
    setCheckError(null);
    try {
      const result = await checkMail(key, form(), pwd);
      if (key !== null) lastChecks.set(key, result);
      if (active.current) setCheck(result);
    } catch (err: unknown) {
      if (!active.current) return;
      setCheck(null);
      setCheckError(typeof err === "string" ? err : t("mail.check.failed"));
    } finally {
      if (active.current) setChecking(false);
    }
  }

  async function save() {
    const passwordToSend = typed();

    setBusy(true);
    setError(null);

    try {
      const saved = await setMailAccount(fresh ? null : id, form(), passwordToSend);
      if (!active.current) return;
      setPassword("");
      if (passwordToSend !== null) setHasPassword(true);
      // Сохранённое проверяем сразу: иначе о неверном пароле человек узнал бы
      // только по карточке, и то двумя словами
      await probe(saved, passwordToSend);
      // Новый ящик получил ключ — дальше это его страница
      if (active.current && fresh) go(`mail/${saved}`);
    } catch (err: unknown) {
      if (active.current) setError(typeof err === "string" ? err : t("mail.settings.saveFailed"));
    } finally {
      if (active.current) setBusy(false);
    }
  }

  async function forget() {
    setBusy(true);
    setError(null);

    try {
      await forgetMailAccount(id);
      if (active.current) go("mail");
    } catch (err: unknown) {
      if (!active.current) return;
      setError(typeof err === "string" ? err : t("mail.settings.forgetFailed"));
      setBusy(false);
    }
  }

  return (
    <div className="rows">
      <div className="field" data-hint={t("mail.settings.imapNote")}>
        <div className="field__head">
          <span className="field__name">{t("settings.source.state")}</span>
          <span className="field__value">
            {t(hasPassword ? "mail.settings.passwordSet" : "mail.settings.passwordUnset")}
          </span>
        </div>
      </div>

      <div className="field">
        <span className="field__name">{t("mail.settings.server")}</span>
        <input
          type="text"
          className="input"
          value={host}
          placeholder="imap.gmail.com"
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setHost(event.target.value)}
        />
      </div>

      <div className="field" data-hint={t("mail.settings.portNote")}>
        <span className="field__name">{t("mail.settings.port")}</span>
        <input
          type="number"
          className="input"
          value={port}
          placeholder="993"
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setPort(event.target.value)}
        />
      </div>

      <div className="field">
        <span className="field__name">{t("mail.settings.login")}</span>
        <input
          type="text"
          className="input"
          value={login}
          placeholder="user@example.com"
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setLogin(event.target.value)}
        />
      </div>

      <div className="field">
        <span className="field__name">{t("mail.settings.password")}</span>
        <input
          type="password"
          className="input"
          value={password}
          placeholder={t(hasPassword ? "mail.settings.keepPassword" : "mail.settings.password")}
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setPassword(event.target.value)}
        />
        <p className="note">{t("mail.settings.passwordNote")}</p>
      </div>

      <div className="field" data-hint={t("mail.settings.mailboxNote")}>
        <span className="field__name">{t("mail.settings.mailbox")}</span>
        <input
          type="text"
          className="input"
          value={mailbox}
          placeholder={mailboxHint !== "" ? mailboxHint : "https://mail.example.com"}
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setMailbox(event.target.value)}
        />
      </div>

      {error !== null && <p className="note note--error">{error}</p>}

      <div className="choices">
        <Button busy={busy} disabled={host.trim() === "" || login.trim() === "" || checking || (fresh && typed() === null)} onClick={save}>
          {t(busy ? "common.saving" : "common.save")}
        </Button>

        {/* Проверить можно и не сохраняя: то, что стоит в форме сейчас */}
        <Button
          variant="outline"
          busy={checking && !busy}
          disabled={busy || checking || host.trim() === "" || login.trim() === "" || (!hasPassword && typed() === null)}
          onClick={() => void probe(fresh ? null : id, typed())}
        >
          {t("mail.check.run")}
        </Button>

        {/* У нового ящика удалять нечего: его ещё нет */}
        {!fresh && (
          <Button variant="outline" disabled={busy || checking} onClick={forget}>
            {t("mail.settings.remove")}
          </Button>
        )}
      </div>

      {checking && <p className="note">{t("mail.check.running")}</p>}
      {!checking && checkError !== null && <p className="note note--error">{checkError}</p>}
      {!checking && check !== null && <CheckResult check={check} host={host.trim()} port={Number(port) || 993} />}
    </div>
  );
}

/** Имя шага в строке «молчит на шаге …». */
const STEP: Record<CheckStage, Key> = {
  resolve: "mail.check.step.resolve",
  connect: "mail.check.step.connect",
  tls: "mail.check.step.tls",
  greeting: "mail.check.step.greeting",
  login: "mail.check.step.login",
  inbox: "mail.check.step.inbox",
};

/** Что случилось на шаге — заголовок отказа. */
const TITLE: Record<CheckStage, Key> = {
  resolve: "mail.check.title.resolve",
  connect: "mail.check.title.connect",
  tls: "mail.check.title.tls",
  greeting: "mail.check.title.greeting",
  login: "mail.check.title.login",
  inbox: "mail.check.title.inbox",
};

/** Что с этим делать — строка под заголовком. */
const HINT: Record<CheckStage, Key> = {
  resolve: "mail.check.hint.resolve",
  connect: "mail.check.hint.connect",
  tls: "mail.check.hint.tls",
  greeting: "mail.check.hint.greeting",
  login: "mail.check.hint.login",
  inbox: "mail.check.hint.inbox",
};

/**
 * Ответ проверки: успех с числом писем — или шаг, на котором встало, что с
 * этим делать и ответ системы или сервера как есть. Код шага (`LOGIN`, `TLS`)
 * не переводится, как и сам ответ: по ним ищут решение.
 */
function CheckResult({ check, host, port }: { check: MailCheck; host: string; port: number }) {
  const t = useText();
  const seconds = (check.millis / 1000).toFixed(1);

  if (check.outcome === "passed") {
    return (
      <div className="mail-check" data-outcome="passed">
        <span className="mail-check__title">{t("mail.check.passedTitle")}</span>
        <span className="mail-check__hint">
          {t("mail.check.passed", { messages: check.messages, unseen: check.unseen, seconds })}
        </span>
      </div>
    );
  }

  return (
    <div className="mail-check" data-outcome="failed">
      <span className="mail-check__title">
        {check.timedOut ? t("mail.check.timeout", { step: t(STEP[check.stage]) }) : t(TITLE[check.stage])}
      </span>
      <span className="mail-check__hint">{t(HINT[check.stage], { host, port })}</span>
      <code className="mail-check__detail" data-no-drag>
        {check.stage.toUpperCase()} · {check.detail}
      </code>
    </div>
  );
}

/**
 * Строка содержимого под темой письма — среди настроек вида карточки.
 *
 * Своей командой и сразу по нажатию: это вид карточки, одна на все ящики, и
 * кнопки «Сохранить» у неё нет. Пока снимок почты не пришёл, галочку не
 * показываем вовсе: погашенная неизвестно чем, она соврала бы о сохранённом.
 */
export function MailPreviewField() {
  const t = useText();
  const mail = useMail();
  const [draft, setDraft] = useState<boolean | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  // Своя правка стоит до ответа Rust: снимок с прежним значением мог быть в пути
  useEffect(() => setDraft(null), [mail?.preview]);

  if (mail === null) return null;
  const preview = draft ?? mail.preview;

  const change = (next: boolean) => {
    setDraft(next);
    setFailure(null);
    setMailPreview(next).catch((reason: unknown) => {
      setDraft(null);
      setFailure(String(reason));
    });
  };

  return (
    <div className="field" data-hint={t("mail.settings.previewNote")}>
      <div className="field__head">
        <span className="field__name">{t("mail.settings.preview")}</span>
        <Check checked={preview} label={t("mail.settings.previewShow")} quiet onChange={change} />
      </div>
      {failure !== null && <p className="note note--error" role="alert">{failure}</p>}
    </div>
  );
}
