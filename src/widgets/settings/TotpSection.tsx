import { useState } from "react";
import { Button } from "../../components/Button";
import {
  addAccount,
  emptyText,
  removeAccount,
  renameAccount,
  reorderAccounts,
  titleOf,
  useAccounts,
} from "../../lib/totp";
import { useText } from "../../lib/i18n";

/**
 * Раздел настроек виджета 2FA: завести запись, переименовать, удалить.
 *
 * Секрет сюда не возвращается никогда — как и пароль почты. Заведённую запись
 * можно только переименовать или удалить; посмотреть её ключ нечем, и это не
 * недоделка, а то же правило, что и у почты.
 */
export function TotpSection() {
  const { accounts, trouble } = useAccounts();
  const t = useText();

  const [input, setInput] = useState("");
  const [issuer, setIssuer] = useState("");
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [removing, setRemoving] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  const [editing, setEditing] = useState<string | null>(null);
  const [draftIssuer, setDraftIssuer] = useState("");
  const [draftLabel, setDraftLabel] = useState("");

  // Перетаскивание порядка: какую запись тянут и над какой она сейчас
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);

  function drop(onto: string) {
    const held = dragging;
    setDragging(null);
    setOver(null);
    if (held === null || held === onto) return;

    const ids = accounts.map((row) => row.id);
    const from = ids.indexOf(held);
    const to = ids.indexOf(onto);
    if (from === -1 || to === -1) return;

    ids.splice(to, 0, ids.splice(from, 1)[0]!);
    reorderAccounts(ids).catch((reason: unknown) => setError(String(reason)));
  }

  async function add() {
    setBusy(true);
    setError(null);

    try {
      await addAccount(input, issuer, label);
      // Поля чистим сразу: секрет не должен оставаться в окне, которое стоит
      // в панели задач и переживает до выхода из приложения
      setInput("");
      setIssuer("");
      setLabel("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveName(id: string) {
    setError(null);
    try {
      await renameAccount(id, draftIssuer, draftLabel);
      setEditing(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function remove(id: string) {
    if (deleting) return;
    setDeleting(true);
    setError(null);
    try {
      await removeAccount(id);
      setRemoving(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="rows">
      <div className="field" data-hint={t("totp.settings.inputNote")}>
        <span className="field__name">{t("totp.settings.input")}</span>
        <input
          type="text"
          className="input"
          value={input}
          placeholder={t("totp.settings.inputHint")}
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setInput(event.target.value)}
        />
      </div>

      <div className="field" data-hint={t("totp.settings.namesNote")}>
        <span className="field__name">{t("totp.settings.issuer")}</span>
        <input
          type="text"
          className="input"
          value={issuer}
          placeholder="GitHub"
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setIssuer(event.target.value)}
        />
      </div>

      <div className="field" data-hint={t("totp.settings.namesNote")}>
        <span className="field__name">{t("totp.settings.label")}</span>
        <input
          type="text"
          className="input"
          value={label}
          placeholder={t("totp.settings.labelHint")}
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => setLabel(event.target.value)}
        />
      </div>

      <div className="field">
        <Button onClick={() => void add()} disabled={busy || input.trim() === ""}>
          {t("totp.settings.add")}
        </Button>
        {error !== null && <p className="note note--error">{error}</p>}
      </div>

      <div className="field">
        {/* Подсказка на заголовке: над записями с их кнопками она мешала бы */}
        <div className="field__head" data-hint={t("totp.settings.entriesHint")}>
          <span className="field__name">{t("totp.settings.entries")}</span>
          <span className="field__value">{accounts.length}</span>
        </div>

        {accounts.length === 0 && trouble !== null ? (
          // Не «записей пока нет»: файл не прочитался или отложен как
          // испорченный, и прежние секреты, возможно, лежат в `totp.bad.json`
          <p className="note note--error" role="alert">
            {t(emptyText(trouble).title)}. {t(emptyText(trouble).hint)}
          </p>
        ) : accounts.length === 0 ? (
          <p className="note">{t("totp.settings.none")}</p>
        ) : (
          <div className="rows rows--tight">
            {accounts.map((row) =>
              removing === row.id ? (
                <div key={row.id} className="field" data-no-drag>
                  <p className="note" role="alert">{t("totp.settings.removeConfirm", { name: titleOf(row) })}</p>
                  <div className="choices">
                    <span className="totp-remove">
                      <Button disabled={deleting} onClick={() => void remove(row.id)}>{t("menu.task.remove")}</Button>
                    </span>
                    <Button disabled={deleting} onClick={() => setRemoving(null)}>{t("common.cancel")}</Button>
                  </div>
                </div>
              ) : editing === row.id ? (
                /* Пока строку правят, тянуть её нельзя: поле ввода и
                   перетаскивание спорили бы за одно и то же движение мыши */
                <div key={row.id} className="field">
                  <input
                    type="text"
                    className="input"
                    value={draftIssuer}
                    placeholder={t("totp.settings.issuer")}
                    spellCheck={false}
                    data-no-drag
                    onChange={(event) => setDraftIssuer(event.target.value)}
                  />
                  <input
                    type="text"
                    className="input"
                    value={draftLabel}
                    placeholder={t("totp.settings.label")}
                    spellCheck={false}
                    data-no-drag
                    onChange={(event) => setDraftLabel(event.target.value)}
                  />
                  <div className="choices">
                    <Button onClick={() => void saveName(row.id)}>{t("common.save")}</Button>
                    <Button onClick={() => setEditing(null)}>{t("common.cancel")}</Button>
                  </div>
                </div>
              ) : (
                <div
                  key={row.id}
                  className="order__row"
                  draggable
                  // Строку тянут для перестановки, а не как ручку окна: без
                  // этого `useWindowDrag` окна настроек начинал перенос окна
                  // на том же сдвиге мыши, что и перетаскивание строки
                  data-no-drag
                  data-dragging={dragging === row.id}
                  data-over={over === row.id && dragging !== row.id}
                  onDragStart={() => setDragging(row.id)}
                  onDragEnd={() => {
                    setDragging(null);
                    setOver(null);
                  }}
                  onDragOver={(event) => {
                    // Без этого браузер не пустит бросок вовсе
                    event.preventDefault();
                    setOver(row.id);
                  }}
                  onDrop={(event) => {
                    event.preventDefault();
                    drop(row.id);
                  }}
                >
                  <span className="order__grip" aria-hidden="true">
                    ⠿
                  </span>
                  <span className="order__name">{titleOf(row)}</span>
                  <span className="choices">
                    <Button
                      onClick={() => {
                        setEditing(row.id);
                        setDraftIssuer(row.issuer);
                        setDraftLabel(row.label);
                      }}
                    >
                      {t("common.rename")}
                    </Button>
                    <Button disabled={deleting} onClick={() => setRemoving(row.id)}>{t("menu.task.remove")}</Button>
                  </span>
                </div>
              ),
            )}
          </div>
        )}
      </div>

      <div className="field">
        <p className="note">{t("totp.settings.dpapiNote")}</p>
      </div>
    </div>
  );
}
