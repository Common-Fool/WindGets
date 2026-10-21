import { useState, useSyncExternalStore } from "react";
import { Button } from "../../components/Button";
import { Check } from "../../components/Check";
import { fileSize } from "../../lib/format";
import {
  openWidgetSettings,
  setAllModules,
  setAllWidgetOption,
  setModule,
  setWidgetOption,
  useSettings,
  type SettingsView,
} from "../../lib/settings";
import { componentsFor, failureText, listOf, removeTools, useTools, type ModuleId } from "../../lib/tools";
import { useText } from "../../lib/i18n";
import { getState, subscribe } from "../../lib/telemetry";
import { gpuPresent } from "../../lib/sensors";

/** Модули со своими скачанными программами. */
const TOOL_MODULES: readonly ModuleId[] = ["converter", "downloader"];

/**
 * Список виджетов: строка на виджет, четыре галочки.
 *
 * Первая галочка, «Вкл.», — это модуль: выключенный не грузится вовсе, ни
 * окна, ни фоновой работы в Rust (`modules.rs`). Выключение и включение
 * мгновенные: окно собирается и разбирается на ходу.
 *
 * Первая строка «Все» переключает настройку сразу у всех виджетов списка.
 * У выключенного модуля остальные три недоступны — применять их не к чему,
 * а галочки создавали бы ощущение работающей настройки. То же и при поднятом
 * рубильнике «спрятать все»: окна спрятаны. Сам модуль под рубильником
 * включить можно: его окно соберётся спрятанным и выйдет вместе со всеми.
 *
 * У нажимаемых виджетов третья ячейка пуста, а не выключена: прозрачность для
 * мыши отключает им приём нажатий целиком, и такой настройки у них нет вовсе.
 * Четвёртая, «замок», есть у всех: она отнимает не мышь, а только право
 * двигать и тянуть окно.
 *
 * ВАЖНО: **общий рубильник «заблокировать положение» гасит весь столбец
 * замка, но галочек в нём не меняет.** Свой флаг общему не подчинён — он
 * просто перекрыт, пока рубильник поднят, и вернётся, как только его опустят.
 */
export function WidgetsSection() {
  const settings = useSettings();
  const t = useText();

  /*
   * Дискретной видеокарты нет — включить её виджет нельзя, показывать ему
   * нечего (этап 57). Живой снимок говорит это сразу; пока датчики спят, его
   * нет, и тогда слово за Rust: он помнит последний снимок (`noGpu`), по нему
   * же «Включить все» пропускает видеокарту (`modules::set_all`). Включённый
   * модуль выключается как всегда.
   *
   * Подписка на один признак, а не на весь снимок: снимок приходит раз в
   * секунду, и таблица перерисовывалась бы с ним впустую.
   */
  const present = useSyncExternalStore(subscribe, () => gpuPresent(getState()));
  const noGpu = present === false || (present === undefined && settings.noGpu);
  const blocked = (widget: SettingsView["widgets"][number]) => noGpu && widget.id === "gpu" && !widget.enabled;

  /*
   * Выключенные только что модули с программами: под таблицей им предлагают
   * удалить то, что больше никому не нужно. Живёт до закрытия окна, на диск
   * не попадает — решение разовое.
   */
  const [offered, setOffered] = useState<ModuleId[]>([]);
  // Отказ включения: окно не собралось, и Rust вернул галочку назад. Очередь
  // переходов держит Rust (номерами, в порядке нажатий — `modules::ticket`),
  // поэтому галочки на время перехода не гасятся — иначе мигали бы все разом
  // на каждом нажатии
  const [error, setError] = useState<string | null>(null);

  const toggleModule = async (id: string, on: boolean) => {
    setError(null);
    try {
      await setModule(id, on);
      const module = TOOL_MODULES.find((entry) => entry === id);
      if (module === undefined) return;
      setOffered((now) => (on ? now.filter((entry) => entry !== module) : [...new Set([...now, module])]));
    } catch (reason) {
      setError(typeof reason === "string" ? reason : t("common.failed"));
    }
  };

  const toggleAllModules = async (on: boolean) => {
    setError(null);
    try {
      await setAllModules(on);
      setOffered(on ? [] : [...TOOL_MODULES]);
    } catch (reason) {
      setError(typeof reason === "string" ? reason : t("common.failed"));
    }
  };

  /*
   * Служебные окна — отладка и витрина — стоят отдельным блоком под чертой, и
   * галочки «Все» их не считают и не трогают: их включают только своей
   * галочкой (12 октября, слово пользователя). Пропускает их и Rust.
   */
  const widgets = settings.widgets.filter((widget) => !widget.service);
  const service = settings.widgets.filter((widget) => widget.service);
  const total = widgets.length;

  // «Все» считается по тем, кого вообще можно включить: погашенная видеокарта
  // оставила бы галочку смешанной навсегда
  const switchable = widgets.filter((widget) => !blocked(widget));
  const enabled = switchable.filter((widget) => widget.enabled).length;
  const allEnabled = switchable.length > 0 && enabled === switchable.length;
  const mixedEnabled = enabled > 0 && enabled < switchable.length;

  const onTop = widgets.filter((widget) => widget.alwaysOnTop).length;
  const allTop = total > 0 && onTop === total;
  const mixedTop = onTop > 0 && onTop < total;

  /*
   * «Сквозь» считается только по тем, у кого эта настройка вообще бывает:
   * нажимаемому виджету прозрачность для мыши отключила бы приём нажатий
   * целиком, и в его строке стоит прочерк. Считай мы их наравне со всеми,
   * галочка «Все» не сошлась бы никогда — полной ей стать было бы нечем.
   */
  const seeThrough = widgets.filter((widget) => !widget.interactive);
  const through = seeThrough.filter((widget) => widget.clickThrough).length;
  const allThrough = seeThrough.length > 0 && through === seeThrough.length;
  const mixedThrough = through > 0 && through < seeThrough.length;
  const throughDisabled = settings.hiddenAll || seeThrough.length === 0;

  const held = widgets.filter((widget) => widget.locked).length;
  const allHeld = total > 0 && held === total;
  const mixedHeld = held > 0 && held < total;
  const lockDisabled = settings.hiddenAll || settings.locked || total === 0;

  const row = (widget: SettingsView["widgets"][number]) => {
          const idle = !widget.enabled || settings.hiddenAll;

          return (
            <div className="grid__row" key={widget.id}>
              <span className="grid__name">{widget.title}</span>

              <Check
                checked={widget.enabled}
                label={blocked(widget) ? t("settings.widgets.noGpu") : t("settings.widgets.showOne", { title: widget.title })}
                disabled={blocked(widget)}
                onChange={(next) => toggleModule(widget.id, next)}
              />

              <Check
                checked={widget.alwaysOnTop}
                label={t("settings.widgets.topOne", { title: widget.title })}
                disabled={idle}
                onChange={(next) => setWidgetOption(widget.id, "top", next)}
              />

              {widget.interactive ? (
                <span className="grid__none" title={t("settings.widgets.clickable")}>
                  —
                </span>
              ) : (
                <Check
                  checked={widget.clickThrough}
                  label={t("settings.widgets.throughOne", { title: widget.title })}
                  disabled={idle}
                  onChange={(next) => setWidgetOption(widget.id, "through", next)}
                />
              )}

              <Check
                checked={widget.locked}
                label={t("settings.widgets.lockOne", { title: widget.title })}
                disabled={idle || settings.locked}
                onChange={(next) => setWidgetOption(widget.id, "lock", next)}
              />

              {/*
                Шестерёнка стоит и у выключенного модуля: окна у него нет, но
                состояние в Rust заведено, и его вид, размер, город и
                программы достижимы и так.
              */}
              <button
                type="button"
                className="grid__gear"
                aria-label={t("settings.widgets.settingsOf", { title: widget.title })}
                title={t("settings.widgets.settingsOf", { title: widget.title })}
                onClick={() => openWidgetSettings(widget.id)}
              >
                {/*
                  Зубцы стоят на самом теле шестерни, а не торчат лучами из
                  кружка: первая редакция рисовала обводкой круг с восемью
                  чёрточками вокруг и читалась как солнышко. Отверстие внутри
                  вырезано вторым контуром по правилу evenodd.
                */}
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    fillRule="evenodd"
                    d="M 6.72 2.65 L 7.14 0.75 L 8.86 0.75 L 9.28 2.65 L 10.87 3.31 L 12.52 2.27 L 13.73 3.48 L 12.69 5.13 L 13.35 6.72 L 15.25 7.14 L 15.25 8.86 L 13.35 9.28 L 12.69 10.87 L 13.73 12.52 L 12.52 13.73 L 10.87 12.69 L 9.28 13.35 L 8.86 15.25 L 7.14 15.25 L 6.72 13.35 L 5.13 12.69 L 3.48 13.73 L 2.27 12.52 L 3.31 10.87 L 2.65 9.28 L 0.75 8.86 L 0.75 7.14 L 2.65 6.72 L 3.31 5.13 L 2.27 3.48 L 3.48 2.27 L 5.13 3.31 Z M 10.5 8 A 2.5 2.5 0 1 0 5.5 8 A 2.5 2.5 0 1 0 10.5 8 Z"
                  />
                </svg>
              </button>
            </div>
          );
  };

  return (
    <div className="rows">
      {settings.hiddenAll && (
        <p className="note">{t("settings.widgets.allHidden")}</p>
      )}

      <div className="grid">
        <div className="grid__head">
          <span />
          <span className="grid__label caps">{t("settings.widgets.show")}</span>
          <span className="grid__label caps">{t("settings.widgets.top")}</span>
          <span className="grid__label caps">{t("settings.widgets.through")}</span>
          <span className="grid__label caps">{t("settings.widgets.lock")}</span>
          <span />
        </div>

        <div className="grid__row grid__row--all">
          <span className="grid__name">{t("settings.widgets.all")}</span>

          <Check
            checked={allEnabled}
            mixed={mixedEnabled}
            label={t("settings.widgets.showAll")}
            disabled={total === 0}
            onChange={toggleAllModules}
          />

          <Check
            checked={allTop}
            mixed={mixedTop}
            label={t("settings.widgets.topAll")}
            disabled={settings.hiddenAll || total === 0}
            onChange={(next) => setAllWidgetOption("top", next)}
          />

          <Check
            checked={allThrough}
            mixed={mixedThrough}
            label={t("settings.widgets.throughAll")}
            disabled={throughDisabled}
            onChange={(next) => setAllWidgetOption("through", next)}
          />

          <Check
            checked={allHeld}
            mixed={mixedHeld}
            label={t("settings.widgets.lockAll")}
            disabled={lockDisabled}
            onChange={(next) => setAllWidgetOption("lock", next)}
          />

          {/* Настройки бывают только у своего виджета: настраивать «всех»
              разом тут нечего, и шестерёнки в этой строке нет */}
          <span />
        </div>

        {widgets.map(row)}

        {service.length > 0 && (
          <h3 className="settings__subheading caps">{t("settings.widgets.service")}</h3>
        )}
        {service.map(row)}
      </div>

      {error && <p className="note note--error" role="alert">{error}</p>}

      {offered.length > 0 && (
        <ToolsOffer modules={offered} settings={settings} onClose={() => setOffered([])} />
      )}

      <p className="note">{t("settings.widgets.columnsNote")}</p>

      {settings.locked && (
        <p className="note">{t("settings.widgets.locksOverridden")}</p>
      )}
    </div>
  );
}

/**
 * Предложение удалить программы выключенных модулей — ровно те, что скачаны и
 * не нужны ни одному включённому. ffmpeg общий у конвертера и загрузчика:
 * выключили один — ffmpeg остаётся другому, и предлагать его нечего.
 *
 * Отдельным компонентом, а не строкой в таблице: стор программ спрашивает
 * Rust, а тот по первому вопросу ищет ffmpeg в системе. Открытие списка
 * виджетов такого поиска не заслуживает — только выключение модуля с
 * программами.
 */
function ToolsOffer({
  modules,
  settings,
  onClose,
}: {
  modules: ModuleId[];
  settings: SettingsView;
  onClose: () => void;
}) {
  const tools = useTools();
  const t = useText();
  const [removing, setRemoving] = useState(false);

  const enabled = (id: string) => settings.widgets.find((row) => row.id === id)?.enabled ?? false;
  const stillNeeded = new Set(
    TOOL_MODULES.filter(enabled).flatMap((module) => componentsFor(tools, module).map((component) => component.id)),
  );

  const removable = modules
    .filter((module) => !enabled(module))
    .flatMap((module) => componentsFor(tools, module))
    .filter((component) => component.presence === "installed" && !stillNeeded.has(component.id))
    .filter((component, index, all) => all.findIndex((other) => other.id === component.id) === index);

  // Удалённое пропадает из списка само: предложение уходит, когда удалять
  // стало нечего, а не по нажатию — иначе отказ («программа занята») было бы
  // некому показать
  if (removable.length === 0) return null;

  const size = removable.reduce((sum, component) => sum + component.diskBytes, 0);
  const failure = tools.failure !== null && removable.some((component) => component.id === tools.failure?.component)
    ? failureText(t, tools.failure.code)
    : null;

  return (
    <div className="field">
      <p className="note">
        {t("settings.widgets.toolsOffer", {
          list: listOf(t, removable.map((component) => component.name)),
          size: fileSize(size),
        })}
      </p>
      <div className="choices">
        <Button
          variant="outline"
          disabled={tools.job !== null || removing}
          onClick={() => {
            setRemoving(true);
            removeTools(removable.map((component) => component.id))
              .catch(() => {})
              .finally(() => setRemoving(false));
          }}
        >
          {t("tools.remove")}
        </Button>
        <Button variant="outline" onClick={onClose}>
          {t("settings.widgets.keepTools")}
        </Button>
      </div>
      {failure !== null && <p className="note note--error" role="alert">{failure}</p>}
    </div>
  );
}
