import { Button } from "../../components/Button";
import { usePlaceSearch } from "../../lib/places";
import { setWeatherPlace, useSettings } from "../../lib/settings";
import { useText } from "../../lib/i18n";
import { WeatherSource } from "./DataSource";

/*
 * Город виджета погоды.
 *
 * Сам поиск живёт в `usePlaceSearch`: то же поле стоит теперь и в карточке
 * погоды, пока город не выбран, — и правила у них общие, а не переписанные
 * заново.
 */

export function WeatherSection() {
  const settings = useSettings();
  const t = useText();
  const chosen = settings.weatherPlace;
  const { query, search, type, now, choose } = usePlaceSearch();

  return (
    <div className="rows">
      <div className="field" data-hint={t("weather.settings.sourceNote")}>
        <div className="field__head">
          <span className="field__name">{t("weather.settings.city")}</span>
          <span className="field__value">{chosen === null ? t("weather.settings.noCity") : chosen.name}</span>
        </div>
        {chosen !== null && chosen.region !== "" && <p className="note">{chosen.region}</p>}
      </div>

      <div className="field">
        <span className="field__name">{t("weather.settings.find")}</span>
        <input
          type="text"
          className="input"
          value={query}
          placeholder={t("weather.place.placeholder")}
          spellCheck={false}
          autoComplete="off"
          data-no-drag
          onChange={(event) => type(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") now();
          }}
        />

        {search.status === "busy" && <p className="note">{t("weather.place.searching")}</p>}
        {search.status === "failed" && <p className="note">{search.reason}</p>}
        {search.status === "done" && search.hits.length === 0 && (
          <p className="note">{t("weather.place.nothing")}</p>
        )}

        {search.status === "done" && search.hits.length > 0 && (
          <div className="places">
            {search.hits.map((place) => (
              <button
                key={`${place.latitude},${place.longitude}`}
                type="button"
                className="places__item"
                onClick={() => choose(place)}
              >
                <span className="places__name">{place.name}</span>
                <span className="places__where">{place.region}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {chosen !== null && (
        <div className="field" data-hint={t("weather.settings.removeNote")}>
          <Button variant="outline" onClick={() => setWeatherPlace(null)}>
            {t("weather.settings.remove")}
          </Button>
        </div>
      )}

      <WeatherSource />
    </div>
  );
}
