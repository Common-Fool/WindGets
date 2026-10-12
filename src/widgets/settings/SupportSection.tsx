import { useState } from "react";
import { Check } from "../../components/Check";
import { ExternalLinkIcon } from "../../components/Icons";
import { copyToClipboard } from "../../lib/copy";
import { setSupportRare, useSettings } from "../../lib/settings";
import { useText } from "../../lib/i18n";
import { openLink } from "../../lib/links";
import { SUPPORT_WAYS, type SupportWay } from "../../lib/support";

/**
 * Поддержать: спасибо и способы задонатить.
 *
 * Список способов живёт в `src/lib/support.json` (типы — в `support.ts`),
 * здесь только его вид. Способы — один блок строк, как связанные ссылки в
 * параметрах Windows: строка целиком нажимается, слева жирное имя с
 * подписью, справа стрелка «в браузер», а у Monobank перед ней QR для телефона. Ссылки открываются тем же путём,
 * что ссылки в заметках (`openLink` → оболочка Windows), адрес кошелька
 * копируется в буфер.
 *
 * Под списком — «Напоминать реже»: то же поле и та же подпись, что у кнопки
 * карточки напоминания (`support_reminder.rs`). Выключить напоминание совсем
 * нельзя — слово пользователя; можно только реже, раз в полгода. Срок в
 * подписи не называем и подсказки не держим — тоже слово пользователя.
 */
export function SupportSection() {
  const t = useText();
  const settings = useSettings();

  return (
    <div className="rows">
      <p className="support__thanks">{t("support.thanks")}</p>

      <div className="support__ways">
        {SUPPORT_WAYS.map((way) => (
          <Way key={way.id} way={way} />
        ))}
      </div>

      <label className="switch">
        <Check
          checked={settings.supportRare}
          label={t("reminder.rare")}
          quiet
          onChange={(next) => setSupportRare(next)}
        />
        <span className="switch__text">
          <span className="switch__name">{t("reminder.rare")}</span>
        </span>
      </label>
    </div>
  );
}

function Way({ way }: { way: SupportWay }) {
  const t = useText();
  const [copied, setCopied] = useState(false);
  const ready = way.value !== "";
  const name = way.name ?? (way.nameKey !== undefined ? t(way.nameKey) : way.id);

  const act = () => {
    if (way.kind === "link") {
      openLink(way.value);
      return;
    }
    void copyToClipboard(way.value).then((done) => {
      setCopied(done);
      window.setTimeout(() => setCopied(false), 2000);
    });
  };

  // Кнопка — само название с подписью: отдельная «Открыть» рядом с ним
  // повторяла бы то же нажатие вторым местом (слово пользователя)
  return (
    <button type="button" className="support__way" disabled={!ready} onClick={act}>
      <span className="support__text">
        <span className="support__name">{name}</span>
        <span className="support__about">{!ready ? t("support.soon") : copied ? t("common.copied") : t(way.about)}</span>
        {ready && way.kind === "copy" && <code className="support__address">{way.value}</code>}
      </span>
      {ready && way.qr !== undefined && <QrCode rows={way.qr} hint={t("support.scan")} />}
      {way.kind === "link" && (
        <span className="support__go">
          <ExternalLinkIcon />
        </span>
      )}
    </button>
  );
}

/**
 * QR из матрицы модулей — один путь по квадратику на модуль.
 *
 * Тёмное на белой плашке при любой теме: камеры банковских приложений
 * инвертированный код читают не все. Поле тишины в четыре модуля — по
 * стандарту, без него код у края плашки ловится хуже.
 */
function QrCode({ rows, hint }: { rows: string[]; hint: string }) {
  const quiet = 4;
  const side = rows.length + quiet * 2;
  let d = "";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === "1") d += `M${x + quiet} ${y + quiet}h1v1h-1z`;
    }
  });
  return (
    <svg
      className="support__qr"
      viewBox={`0 0 ${side} ${side}`}
      shapeRendering="crispEdges"
      role="img"
      aria-label={hint}
      data-hint={hint}
    >
      <rect width={side} height={side} fill="#fff" />
      <path d={d} fill="#000" />
    </svg>
  );
}
