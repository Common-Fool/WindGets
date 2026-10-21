import { useText } from "../../lib/i18n";
import { openLink } from "../../lib/links";
import { ExternalLinkIcon } from "../../components/Icons";

/** Одна кнопка перехода: при обрезке переезжает из шапки вниз письма. */
export function LetterBrowser({ link, className, caption = false }: { link: string; className: string; caption?: boolean }) {
  const t = useText();
  return (
    <button type="button" className={className} title={t("mail.openBrowser")} aria-label={t("mail.openBrowser")}
      onClick={() => openLink(link)}>
      <ExternalLinkIcon />
      {caption && <span>{t("mail.openBrowser")}</span>}
    </button>
  );
}

export function LetterIncomplete({ link }: { link: string | null }) {
  const t = useText();
  return (
    <div className="mail__incomplete" role="status">
      <span>{t("mail.bodyTruncated")}</span>
      {link !== null && <LetterBrowser link={link} className="mail__incomplete-link" caption />}
    </div>
  );
}
