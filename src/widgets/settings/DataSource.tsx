import { Button } from "../../components/Button";
import { useText, type Key } from "../../lib/i18n";
import { openLink } from "../../lib/links";

/*
 * Откуда виджет берёт данные и что с ними происходит — на странице виджета в
 * настройках, а не в карточке (15 октября, слово пользователя: на сам виджет
 * ничего не добавлять).
 *
 * Требуют этого условия сервисов (проверка лицензий 15 октября): Open-Meteo —
 * подписи «Weather data by Open-Meteo.com» со ссылкой и лицензии данных
 * CC BY 4.0; M-Lab — ссылок на политики. Что M-Lab публикует IP-адрес, здесь
 * не пересказываем (слово пользователя): это сказано в политиках по ссылкам.
 */

type Link = { label: Key; url: string };

const PRIVACY = "https://github.com/Common-Fool/WindGets/blob/main/PRIVACY.md";

function DataSource({ text, links }: { text: Key[]; links: Link[] }) {
  const t = useText();
  return (
    <section className="field">
      <h3 className="settings__subheading caps">{t("source.heading")}</h3>
      {text.map((key) => <p key={key} className="note">{t(key)}</p>)}
      <div className="choices">
        {links.map((link) => (
          <Button key={link.url} variant="outline" onClick={() => openLink(link.url)}>
            {t(link.label)}
          </Button>
        ))}
      </div>
    </section>
  );
}

export function WeatherSource() {
  return (
    <DataSource
      text={["source.weather.attribution", "source.weather.license"]}
      links={[
        { label: "source.weather.site", url: "https://open-meteo.com/" },
        { label: "source.weather.ccBy", url: "https://creativecommons.org/licenses/by/4.0/" },
      ]}
    />
  );
}

export function SpeedtestSource() {
  return (
    <DataSource
      text={["source.speedtest.who"]}
      links={[
        { label: "source.speedtest.mlabPrivacy", url: "https://www.measurementlab.net/privacy/" },
        { label: "source.speedtest.mlabRules", url: "https://www.measurementlab.net/aup/" },
        { label: "source.privacy", url: PRIVACY },
      ]}
    />
  );
}
