import { useText } from "../../lib/i18n";
import { LetterBrowser, LetterIncomplete } from "./LetterBrowser";
import { fileSize, formatLetterTime } from "../../lib/format";
import { copyToClipboard } from "../../lib/copy";
import { deleteLetter, markMailRead, type LetterBody } from "../../lib/mail";
import { Linked } from "../../components/Linked";
import { BoxCheckedIcon, BoxIcon, CheckIcon, ClipIcon, CloseIcon, EraseIcon, RefreshIcon } from "../../components/Icons";
import type { LetterPopupProps, LetterRowProps } from "./MailWidget";

/** A — колонки; B — тема и метаданные. Команды и ключи те же, что у Classic. */
export function ReferenceLetterRow({ letter, onSelect, act, busy, picked, anyPicked, onPick, layout = 0 }: LetterRowProps) {
  const t = useText();
  const from = <span className="mail-ref__from" title={letter.from}>{letter.from}</span>;
  const time = <span className="mail-ref__time tabular">{formatLetterTime(letter.at, t.locale)}</span>;
  const actions = (
    <div className="mail-ref__actions" data-busy={busy}>
      {busy ? (
        <span className="mail-ref__button spinner" title={t("mail.working")}><RefreshIcon /></span>
      ) : (
        <>
          {!letter.seen && (
            <button type="button" className="mail-ref__button" title={t("mail.markRead")} aria-label={t("mail.markRead")}
              onClick={(event) => { event.stopPropagation(); act(letter, markMailRead(letter.account, [letter.uid], letter.revision, letter.uidValidity), "not marked", "read"); }}>
              <CheckIcon />
            </button>
          )}
          <button type="button" className="mail-ref__button mail-ref__button--delete" title={t("mail.delete")} aria-label={t("mail.delete")}
            onClick={(event) => { event.stopPropagation(); act(letter, deleteLetter(letter.account, [letter.uid], letter.revision, letter.uidValidity), "not deleted", "delete"); }}>
            <EraseIcon />
          </button>
        </>
      )}
    </div>
  );

  return (
    <li className="mail-ref__row" data-layout={layout === 1 ? 1 : 0} data-picked={picked} data-seen={letter.seen} data-selecting={anyPicked}
      onClick={() => onSelect(letter)}>
      <span className="mail-ref__mark">
        <button type="button" className="mail-ref__pick" data-on={picked} title={picked ? t("mail.deselect") : t("mail.select")}
          aria-label={picked ? t("mail.deselect") : t("mail.select")}
          onClick={(event) => { event.stopPropagation(); onPick(!picked); }} onDoubleClick={(event) => event.stopPropagation()}>
          {picked ? <BoxCheckedIcon /> : <BoxIcon />}
        </button>
        <span className="mail-ref__dot" data-seen={letter.seen} />
      </span>
      <span className="mail-ref__subject" title={letter.subject}>{letter.subject}</span>
      {layout === 1 ? <div className="mail-ref__meta">{from}{time}{actions}</div> : <>{from}{time}{actions}</>}
      {letter.preview !== null && letter.preview !== "" && <span className="mail-ref__preview">{letter.preview}</span>}
    </li>
  );
}

function ReferenceAddress({ label, value }: { label: string; value: string }) {
  const t = useText();
  return (
    <div className="mail-ref__address">
      <span className="mail-ref__address-label">{label}</span>
      <button type="button" className="mail-ref__address-value" title={t("mail.copyAddress")} onClick={() => void copyToClipboard(value)}>{value}</button>
    </div>
  );
}

type PopupContentProps = LetterPopupProps & { body: LetterBody | null; loading: boolean; failed: string | null };

/** Тело и загрузка живут в прежнем LetterPopup и переживают смену визуала. */
export function ReferenceLetterPopupContent({ letter, onClose, busy, onMarkRead, body, loading, failed }: PopupContentProps) {
  const t = useText();
  return (
    <div className="mail-ref__overlay" onClick={onClose}>
      <div className="mail-ref__popup" onClick={(event) => event.stopPropagation()}>
        <div className="mail-ref__popup-tools">
          {busy ? <span className="mail-ref__button spinner" title={t("mail.working")}><RefreshIcon /></span> : (
            <button type="button" className="mail-ref__button" title={letter.seen ? t("mail.alreadyRead") : t("mail.markRead")}
              aria-label={letter.seen ? t("mail.alreadyRead") : t("mail.markRead")} disabled={letter.seen} onClick={onMarkRead}>
              <CheckIcon />
            </button>
          )}
          {letter.link !== null && !body?.truncated && <LetterBrowser link={letter.link} className="mail-ref__button" />}
          <button type="button" className="mail-ref__button mail-ref__close" title={t("settings.close")} aria-label={t("settings.close")} onClick={onClose}><CloseIcon /></button>
        </div>
        <div className="mail-ref__reader" data-no-drag>
          <h3 className="mail-ref__popup-subject">{letter.subject}</h3>
          <div className="mail-ref__popup-meta">
            <span className="mail-ref__from" title={letter.from}>{letter.from}</span>
            <span className="mail-ref__popup-time tabular">{formatLetterTime(letter.at, t.locale)}</span>
          </div>
          {letter.fromAddress !== null && <ReferenceAddress label={t("mail.from")} value={letter.fromAddress} />}
          {letter.to.length > 0 && <ReferenceAddress label={t("mail.to")} value={letter.to.join(", ")} />}
          <div className="mail-ref__body">
            {loading ? <div className="mail-ref__body-state"><span className="spinner"><RefreshIcon /></span></div> : failed ? (
              <div className="mail-ref__body-state">{failed}</div>
            ) : body !== null && body.text !== null && body.text.trim() !== "" ? (
              <Linked text={body.text} />
            ) : <div className="mail-ref__body-state">{t("mail.popupEmpty")}</div>}
          </div>
          {body !== null && body.attachments.length > 0 && (
            <ul className="mail-ref__files">
              {body.attachments.map((file, index) => <li className="mail-ref__file" key={`${file.name}-${index}`}>
                <span className="mail-ref__file-icon"><ClipIcon /></span>
                <span className="mail-ref__file-name">{file.name}</span>
                <span className="mail-ref__file-size tabular">{fileSize(file.size)}</span>
              </li>)}
            </ul>
          )}
          {body?.truncated && <LetterIncomplete link={letter.link} />}
        </div>
      </div>
    </div>
  );
}
