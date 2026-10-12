import { usePlaceSearch } from "../../lib/places";
import { PinIcon } from "./icons";
import { useText } from "../../lib/i18n";

/**
 * Карточка погоды, пока город не выбран: поле поиска прямо в ней.
 *
 * ВАЖНО: **это не сокращённый путь к настройкам, а весь путь целиком.** Прежде
 * карточка показывала прочерки и строку «Настройки → Погода» — то есть на
 * первом запуске виджет сообщал, что он не работает, и отсылал искать окно, о
 * существовании которого ещё никто не знает.
 *
 * Поиск тот же, что в разделе настроек, и живёт он в `usePlaceSearch`: правила
 * у поля одни, а не две расходящиеся копии.
 */
export function PlacePanel() {
  const { query, search, type, now, choose } = usePlaceSearch();
  const t = useText();

  return (
    <div className="place">
      <span className="place__icon">
        <PinIcon />
      </span>
      <p className="place__lead">{t("weather.place.lead")}</p>

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

      {/* Список прокручивается, а не растит окно: восемь находок в карточку
          не помещаются, а вырасти ей тут не под что — панель эта временная,
          и следующее, что случится, это выбор города */}
      <div className="place__found">
        {search.status === "busy" && <p className="place__note">{t("weather.place.searching")}</p>}
        {search.status === "failed" && (
          <p className="place__note place__note--danger">{search.reason}</p>
        )}
        {search.status === "done" && search.hits.length === 0 && (
          <p className="place__note">{t("weather.place.nothing")}</p>
        )}
        {search.status === "idle" && (
          <p className="place__note">{t("weather.place.offlineNote")}</p>
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
    </div>
  );
}
