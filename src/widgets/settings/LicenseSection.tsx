import { invoke } from "@tauri-apps/api/core";
import { Button } from "../../components/Button";
import { ExternalLinkIcon } from "../../components/Icons";
import { useText, type Key } from "../../lib/i18n";
import { openLink } from "../../lib/links";
import { useSettings } from "../../lib/settings";
import credits from "../../lib/credits.json";
import licenseText from "../../../LICENSE?raw";

/*
 * Лицензия: своя и чужая.
 *
 * Своя — MIT, текст берётся из файла LICENSE в корне репозитория: один
 * источник, который едет и в установщик. Текст лицензии не переводится —
 * переведённая лицензия юридически уже другой документ.
 *
 * Чужое в поставке — сводкой: три части, сколько в каждой компонентов и под
 * какими лицензиями они все. Поимённо, с версиями, авторами и текстами —
 * только в `THIRD-PARTY-NOTICES.txt`, кнопкой. Полсотни строк с «MIT» в
 * каждой сняты с живой проверки по слову пользователя (этап 52): так
 * делают и остальные — у VS Code файл рядом с программой, у Chrome отдельная
 * страница, — а лицензии требуют текстов в поставке, не списка в настройках.
 *
 * Скачиваемые программы и сервисы — поимённо: их шесть, в поставку они не
 * входят, и о каждом есть что сказать.
 *
 * Всё это собирает `npm run notices` из манифестов пакетов, так что лицензии
 * тут не набраны руками и с пакетами не разойдутся. Свежесть держит тест
 * `notices::tests::notices_are_fresh`.
 */

type Credit = { id?: string; name: string; license: string; url: string };

/** Части поставки: что это за код, словами каталога. */
const PARTS: Record<string, Key> = {
  sidecar: "license.part.sidecar",
  rust: "license.part.rust",
  web: "license.part.web",
};

/** Пояснения к отдельным сервисам: их условия — не только лицензия. */
const NOTES: Record<string, Key> = {
  openmeteo: "license.note.openmeteo",
  mlab: "license.note.mlab",
  spotify: "license.note.spotify",
};

export function LicenseSection() {
  const t = useText();
  // Версия — из Cargo.toml через снимок настроек: по ней сверяют выпуски и
  // называют сборку в сообщении об ошибке
  const { version } = useSettings();

  return (
    <div className="rows">
      <div className="field">
        <div className="field__head">
          <span className="field__name">{version === "" ? "Windgets" : `Windgets ${version}`}</span>
          <span className="field__value">MIT</span>
        </div>
        <p className="note">{t("license.own")}</p>
        <details className="license-text">
          <summary>{t("license.showText")}</summary>
          <pre data-no-drag>{licenseText}</pre>
        </details>
      </div>

      <section className="field">
        <h3 className="settings__subheading caps">{t("license.group.bundled")}</h3>
        <div className="credits">
          {credits.parts.map((part) => {
            const title = PARTS[part.id];
            return (
              <div key={part.id} className="credit">
                <span className="credit__name">{title === undefined ? part.id : t(title)}</span>
                <span className="credit__license">
                  {t("license.andMore", { name: part.lead, count: part.count - 1 })}
                </span>
              </div>
            );
          })}
        </div>
        <p className="note">{t("license.bundledNote", { list: credits.licenses.join(", ") })}</p>
        <div className="choices" data-hint={t("license.fullHint")}>
          <Button variant="outline" onClick={() => void invoke("open_notices").catch(() => {})}>
            {t("license.full")}
          </Button>
        </div>
      </section>

      <section className="field">
        <h3 className="settings__subheading caps">{t("license.group.programs")}</h3>
        <p className="note">{t("license.about.programs")}</p>
        <div className="credits">
          {(credits.programs as Credit[]).map((item) => <CreditRow key={item.name} item={item} />)}
        </div>
      </section>

      <section className="field">
        <h3 className="settings__subheading caps">{t("license.group.services")}</h3>
        <div className="credits">
          {(credits.services as Credit[]).map((item) => <CreditRow key={item.name} item={item} />)}
        </div>
      </section>
    </div>
  );
}

function CreditRow({ item }: { item: Credit }) {
  const t = useText();
  const note = item.id !== undefined ? NOTES[item.id] : undefined;

  return (
    <div className="credit">
      <span className="credit__name">{item.name}</span>
      <span className="credit__license">{item.license}</span>
      <button type="button" className="credit__link" title={item.url} aria-label={item.url} onClick={() => openLink(item.url)}>
        <ExternalLinkIcon />
      </button>
      {note !== undefined && <span className="credit__note">{t(note)}</span>}
    </div>
  );
}
