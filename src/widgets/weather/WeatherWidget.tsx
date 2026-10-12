import { useEffect, useRef, useState, type RefObject } from "react";
import {
  accentOf,
  degrees,
  refreshWeather,
  useWeather,
  type Ahead,
  type Hour,
  type WeatherState,
  type Report,
  type Sky,
} from "../../lib/weather";
import { useWindowDrag } from "../../lib/drag";
import { useCardClick } from "../../lib/gestures";
import { useWindowResized } from "../../lib/resized";
import { useContentHeight, useWindowGrowth } from "../../lib/grow";
import { useLook, useScale } from "../../lib/look";
import { useHeight } from "../../lib/size";
import { useWidgetView, useWidgetSize } from "../../lib/view";
import { NO_VALUE } from "../../lib/format";
import { GlassCard } from "../../components/GlassCard";
import { RefreshIcon } from "../../components/Icons";
import { WeatherIcon } from "./icons";
import { PlacePanel } from "./PlacePanel";
import "./weather.css";
import "./weather-reference.css";
import { useText, type Text } from "../../lib/i18n";

/**
 * Что раскрыто под рядом дней.
 *
 * `TODAY` — нынешний день по часам, всё прочее — дата дня из ряда. Даты хватает
 * и как признака выбранного: клетка подсвечивает себя по ней, и держать рядом
 * второй номер незачем.
 */
const TODAY = "today";

/**
 * Отступ панели от ряда дней при кегле 100 %.
 *
 * Считать его приходится отдельно: высоту панели берём у неё самой, а поле
 * снаружи в эту высоту не входит. Черта и поле под ней — входят.
 */
const PANEL_GAP = 8;

/**
 * Сколько ждём ответа на нажатие кнопки обновления.
 *
 * Значок крутится, пока не придёт погода. Ответа может не прийти вовсе — сеть
 * молчит, запрос отваливается по своему сроку, — и вечно крутящийся значок
 * выглядел бы поломкой. Срок тут заведомо больше того, что стоит у запроса.
 */
const REFRESH_TIMEOUT = 15000;

/**
 * Имя дня в ряду.
 *
 * Первый называется словом: «Завтра» понятнее дня недели ровно для ближайшего
 * дня — дальше слов не хватает, и начинаются «Ср» и «Чт».
 *
 * ВАЖНО: **полночь тут местная, а не по Гринвичу.** Голую дату «2026-09-03»
 * браузер читает как UTC, и западнее Гринвича день недели съезжал бы на
 * предыдущий. Часовой пояс города при этом ни при чём: день недели — свойство
 * самой даты.
 */
function dayName(t: Text, day: Ahead, at: number): string {
  if (at === 0) return t("weather.tomorrow");

  // Дата пришла от чужого сервиса: не тот формат — дата невалидна, и её
  // показываем как есть
  const parsed = new Date(`${day.date}T00:00`);
  if (Number.isNaN(parsed.getTime())) return day.date;

  // Имя дня — у языка, а не своим списком: «Пн», «Mon», «lun», «周一». С
  // заглавной, как стояло прежде: у русского и испанского Intl даёт строчные
  const name = new Intl.DateTimeFormat(t.locale, { weekday: "short" }).format(parsed);
  return name.charAt(0).toLocaleUpperCase(t.locale) + name.slice(1);
}

/** Проценты вероятности осадков или прочерк. */
function rain(value: number | null | undefined): string {
  return value === null || value === undefined ? NO_VALUE : `${value} %`;
}

/** Метры в секунду с десятой долей или прочерк. */
function wind(t: Text, value: number | null | undefined): string {
  return value === null || value === undefined
    ? NO_VALUE
    : t("weather.wind", { speed: value.toFixed(1) });
}

/** Строка часа в раскрытой панели: время, значок, градусы, осадки, ветер. */
function HourRow({ hour }: { hour: Hour }) {
  const t = useText();
  return (
    <li className="weather__hour">
      <span className="weather__hour-time tabular">{hour.at}</span>
      <span className="weather__hour-icon">
        <WeatherIcon sky={hour.sky} day={hour.day} />
      </span>
      <span className="weather__hour-degrees tabular">
        {degrees(hour.temperature) ?? NO_VALUE}
      </span>
      <span className="weather__hour-rain tabular">{rain(hour.precipitation)}</span>
      <span className="weather__hour-wind tabular">{wind(t, hour.wind)}</span>
    </li>
  );
}

/**
 * Клетка дня: значок, имя, макс и мин.
 *
 * Не кнопка, хотя и нажимается: за кнопку окно не потащишь, а тянут виджеты за
 * что угодно, кроме органов управления. Нажатие ловит `useCardClick` и узнаёт
 * клетку по `data-pick`.
 */
function DayCell({ day, at, open }: { day: Ahead; at: number; open: boolean }) {
  const t = useText();
  return (
    <div className="weather__cell" data-pick={day.date} data-open={open}>
      <span className="weather__cell-icon">
        {/* Всегда дневной: сутки целиком, и ночным значком они выглядели бы
            прогнозом на вечер */}
        <WeatherIcon sky={day.sky} day />
      </span>
      <span className="weather__cell-name">{dayName(t, day, at)}</span>
      <span className="weather__cell-max tabular">{degrees(day.day.max) ?? NO_VALUE}</span>
      <span className="weather__cell-min tabular">{degrees(day.day.min) ?? NO_VALUE}</span>
    </div>
  );
}

/**
 * Строка над основным блоком: обычно город, а при беде — то, что мешает.
 *
 * Место одно и то же в любом состоянии: карточка не должна прыгать оттого, что
 * ответ не пришёл. Слова о погоде («Пасмурно») тут больше нет — значок в
 * основном блоке говорит то же самое и крупнее.
 */
function caption(t: Text, state: WeatherState): string {
  switch (state.status) {
    case "loading":
      return t("weather.loading");
    case "failed":
      return `${state.place} · ${state.reason}`;
    case "ready":
      // Затянувшееся молчание приписываем к городу: отдельной строкой оно
      // сдвигало бы всё, что ниже, на каждой осечке сети
      return state.stale ? `${state.place} · ${t("weather.error.offline")}` : state.place;
    default:
      return t("weather.noCity");
  }
}


type WeatherRenderProps = {
  state: WeatherState;
  report: Report | null;
  sky: Sky;
  open: string | null;
  hours: Hour[];
  spinning: boolean;
  refresh: () => void;
};

/** Исходная разметка Claude и его рост живут вместе; состояние остаётся у родителя. */
function ClaudeWeather({ state, report, sky, open, hours, spinning, refresh, leavingForSetup }:
  WeatherRenderProps & { leavingForSetup: RefObject<boolean> }) {
  const t = useText();
  const growth = useWindowGrowth();
  const geometry = useScale();
  const panel = useRef<HTMLUListElement>(null);
  const measured = useHeight(panel);

  /*
   * Высота окна догоняет содержимое панели.
   *
   * ВАЖНО: **высоту берём у самой панели, а не из измеренной.** Измеренная в
   * этот момент прошлая: наблюдатель сработает позже, и окно успело бы встать
   * по высоте прежнего дня, а потом дёрнуться во вторую сторону. В зависимости
   * она нужна затем, чтобы этот заход случился ещё раз, когда содержимое
   * приедет или сменится.
   */
  useEffect(() => {
    const box = panel.current;
    if (box === null) {
      growth.close();
      return;
    }

    growth.open(box.offsetHeight + PANEL_GAP * geometry);
  }, [hours.length, measured, geometry, growth]);

  // Уход к первому запуску закрывает прежнюю панель; смена дизайна уже поставила новую основу.
  useEffect(() => () => {
    if (leavingForSetup.current) growth.close();
  }, [growth, leavingForSetup]);
  return (
          <div className="weather">
            <div className="weather__head">
              <span className="weather__place" data-error={state.status === "failed"}>
                {caption(t, state)}
              </span>
              <button
                type="button"
                className="refresh"
                data-spinning={spinning}
                title={t("weather.refresh")}
                onClick={refresh}
              >
                <RefreshIcon />
              </button>
            </div>

            <div className="weather__now" data-pick={TODAY} data-open={open === TODAY}>
              <span className="weather__icon">
                <WeatherIcon sky={sky} day={report?.day ?? true} />
              </span>

              {/* Единицы рядом с числом нет: Цельсий тут подразумевается, а
                  «°C» у крупной цифры оказалось бы единственной подписью */}
              <span className="weather__degrees tabular">
                {degrees(report?.temperature ?? null) ?? NO_VALUE}
              </span>

              <div className="weather__figures">
                <span className="weather__extra">
                  {t("weather.precipitation")} <span className="tabular">{rain(report?.precipitation)}</span>
                </span>
                <span className="weather__extra">
                  {t("weather.windLabel")} <span className="tabular">{wind(t, report?.wind)}</span>
                </span>
              </div>
            </div>

            <div className="weather__row">
              {(report?.days ?? []).map((day, at) => (
                <DayCell key={day.date} day={day} at={at} open={open === day.date} />
              ))}
            </div>

            {hours.length > 0 && (
              <ul className="weather__panel" ref={panel}>
                {hours.map((hour) => (
                  <HourRow key={hour.at} hour={hour} />
                ))}
              </ul>
            )}
          </div>
  );
}

/** Значения текущей погоды. Цель остаётся обычным блоком, за который тянут окно. */
function ReferenceNow({ report, sky, open }: Pick<WeatherRenderProps, "report" | "sky" | "open">) {
  const t = useText();
  return (
    <div className="weather-reference__now" data-pick={TODAY} data-open={open === TODAY}>
      <span className="weather-reference__now-label">{t("weather.current")}</span>
      <div className="weather-reference__current">
        <span className="weather-reference__icon"><WeatherIcon sky={sky} day={report?.day ?? true} /></span>
        <span className="weather-reference__degrees tabular">{degrees(report?.temperature ?? null) ?? NO_VALUE}</span>
      </div>
      <div className="weather-reference__figures">
        <div className="weather-reference__figure">
          <span title={t("weather.precipitation")}>{t("weather.precipitation")}</span>
          <span className="tabular">{rain(report?.precipitation)}</span>
        </div>
        <div className="weather-reference__figure">
          <span title={t("weather.windLabel")}>{t("weather.windLabel")}</span>
          <span className="tabular">{wind(t, report?.wind)}</span>
        </div>
      </div>
    </div>
  );
}

/** Пять будущих дней: те же данные и тот же data-pick в обоих видах GPT. */
function ReferenceDays({ report, open }: Pick<WeatherRenderProps, "report" | "open">) {
  const t = useText();
  if (report === null) return null;
  return (
    <div className="weather-reference__days">
      {report.days.map((day, at) => (
        <div className="weather-reference__day" data-pick={day.date} data-open={open === day.date} key={day.date}>
          <span className="weather-reference__day-name" title={dayName(t, day, at)}>{dayName(t, day, at)}</span>
          <span className="weather-reference__day-icon"><WeatherIcon sky={day.sky} day /></span>
          <span className="weather-reference__day-max tabular">{degrees(day.day.max) ?? NO_VALUE}</span>
          <span className="weather-reference__day-min tabular">{degrees(day.day.min) ?? NO_VALUE}</span>
        </div>
      ))}
    </div>
  );
}

/** Таблица A и сетка B сохраняют все пять полей каждого часа. */
function ReferenceHour({ hour }: { hour: Hour }) {
  const t = useText();
  return (
    <li className="weather-reference__hour">
      <span className="weather-reference__hour-time tabular">{hour.at}</span>
      <span className="weather-reference__hour-icon"><WeatherIcon sky={hour.sky} day={hour.day} /></span>
      <span className="weather-reference__hour-degrees tabular">{degrees(hour.temperature) ?? NO_VALUE}</span>
      <span className="weather-reference__hour-rain tabular">{rain(hour.precipitation)}</span>
      <span className="weather-reference__hour-wind tabular">{wind(t, hour.wind)}</span>
    </li>
  );
}

function ReferenceWeather({
  state, report, sky, open, hours, spinning, refresh, view, content,
}: WeatherRenderProps & { view: number; content: RefObject<HTMLDivElement | null> }) {
  const t = useText();
  const now = <ReferenceNow report={report} sky={sky} open={open} />;
  const days = <ReferenceDays report={report} open={open} />;
  return (
    <div className="weather-reference" data-ready={report !== null} ref={content}>
      <div className="weather-reference__head">
        <span className="weather-reference__place" data-error={state.status === "failed"} title={caption(t, state)}>
          {caption(t, state)}
        </span>
        <button type="button" className="weather-reference__refresh" data-spinning={spinning}
          title={t("weather.refresh")} aria-label={t("weather.refresh")} onClick={refresh}>
          <RefreshIcon />
        </button>
      </div>
      <div className="weather-reference__base">
        {view === 1 ? <>{days}{now}</> : <>{now}{days}</>}
      </div>
      {hours.length > 0 && (
        <ul className="weather-reference__panel">
          {hours.map((hour) => <ReferenceHour hour={hour} key={hour.at} />)}
        </ul>
      )}
    </div>
  );
}

export function WeatherWidget() {
  const state = useWeather();
  const t = useText();
  const [open, setOpen] = useState<string | null>(null);
  const [spinning, setSpinning] = useState(false);
  const { effectiveDesign, scale } = useLook();
  const reference = effectiveDesign === "reference";
  const view = useWidgetView() === 1 ? 1 : 0;
  const size = useWidgetSize() === 1 ? 1 : 0;
  const widget = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  useWindowDrag();

  const report = state.status === "ready" ? state.report : null;
  const sky = report?.sky ?? "overcast";
  const setup = state.status === "noPlace";
  const leavingForSetup = useRef(false);
  leavingForSetup.current = !reference && setup;

  // Один набор состояния на оба дизайна: переключение не закрывает выбранные часы.
  useCardClick((target) => {
    if (setup) return;
    const picked = target?.closest("[data-pick]")?.getAttribute("data-pick") ?? null;
    setOpen((now) => (picked === now ? null : picked));
  });

  // Готовый размер по-прежнему сворачивает раскрытие; смена дизайна этот event не шлёт.
  useWindowResized(() => setOpen(null));

  useEffect(() => {
    if (open === null || open === TODAY || report === null) return;
    if (!report.days.some((day) => day.date === open)) setOpen(null);
  }, [open, report]);

  useEffect(() => { setSpinning(false); }, [state]);
  useEffect(() => {
    if (!spinning) return;
    const timer = window.setTimeout(() => setSpinning(false), REFRESH_TIMEOUT);
    return () => window.clearTimeout(timer);
  }, [spinning]);

  function refresh() {
    setSpinning(true);
    void refreshWeather();
  }

  const hours: Hour[] = open === null ? [] : open === TODAY
    ? (report?.hours ?? [])
    : (report?.days.find((day) => day.date === open)?.windows ?? []);

  /*
   * PlacePanel остаётся на общем месте дерева React: при смене дизайна поле,
   * дебаунс и результаты не монтируются заново. Меряем уже существующий узел,
   * высота которого в GPT фиксирована по выбранной основе, а находки прокручиваются.
   */
  useEffect(() => {
    if (reference && setup) content.current = widget.current?.querySelector<HTMLDivElement>(".place") ?? null;
  }, [reference, setup, view, size]);

  // У GPT одна подгонка всей карточки. ClaudeWeather один владеет старым growth.
  useContentHeight(content, reference, `${view}|${size}|${setup}|${open}|${state.status}|${hours.length}|${scale}|${t.locale}`);

  const rendering: WeatherRenderProps = { state, report, sky, open, hours, spinning, refresh };
  return (
    <div className={reference ? "widget weather-widget--reference" : "widget"}
      data-weather-view={reference ? view : undefined} data-weather-size={reference ? size : undefined} ref={widget}>
      <GlassCard accent={report === null ? "neutral" : accentOf(sky)}>
        {setup ? <PlacePanel /> : reference
          ? <ReferenceWeather {...rendering} view={view} content={content} />
          : <ClaudeWeather {...rendering} leavingForSetup={leavingForSetup} />}
      </GlassCard>
    </div>
  );
}
