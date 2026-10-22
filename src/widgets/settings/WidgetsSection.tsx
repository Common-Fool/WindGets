import { useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { Button } from "../../components/Button";
import { CheckIcon, CloseIcon } from "../../components/Icons";
import { fileSize } from "../../lib/format";
import { setModule, useSettings, type SettingsView, type WidgetRow } from "../../lib/settings";
import { componentsFor, failureText, listOf, removeTools, useTools, type ModuleId } from "../../lib/tools";
import { useText, type Key } from "../../lib/i18n";
import { useLook } from "../../lib/look";
import { getState, subscribe } from "../../lib/telemetry";
import { gpuPresent } from "../../lib/sensors";
import { PREVIEWED } from "../../preview/list";
import { HEIGHT, READY, TO_FRAME, type PreviewState, type ToFrame, type ToHost } from "../../preview/protocol";
import { WIDGET_ICONS } from "../icons";
import { Navigate, Picked } from "./nav";

/** Модули со своими скачанными программами. */
const TOOL_MODULES: readonly ModuleId[] = ["converter", "downloader"];

/** Ключ описания виджета в каталоге. Есть у каждого — держит `every_widget_has_a_description`. */
const about = (id: string) => `widgetAbout.${id}` as Key;

/**
 * Раздел «Виджеты»: галерея карточек по образцу Themia (22 октября, слово
 * пользователя). Слева карточки — значок, имя, описание; справа живое превью
 * выбранной.
 *
 * Включённые стоят сверху, остальные ниже, служебные окна — своей группой в
 * самом конце. Кнопка на карточке включает модуль (галочка) или выключает его
 * (крестик), и карточка переезжает в свою группу. Выключенный модуль не
 * грузится вовсе — ни окна, ни фоновой работы в Rust (`modules.rs`).
 *
 * Двойной щелчок по карточке (и Enter) открывает страницу виджета в дереве
 * колонки; то же делает «Настроить» под превью. «Поверх» и «замок» живут
 * кнопками в самом дереве, у включённых виджетов.
 */
export function WidgetsSection() {
  const settings = useSettings();
  const t = useText();
  const navigate = useContext(Navigate);
  const [picked, pick] = useContext(Picked);

  /*
   * Дискретной видеокарты нет — включить её виджет нельзя, показывать ему
   * нечего (этап 57). Живой снимок говорит это сразу; пока датчики спят, его
   * нет, и тогда слово за Rust: он помнит последний снимок (`noGpu`).
   * Включённый модуль выключается как всегда.
   *
   * Подписка на один признак, а не на весь снимок: снимок приходит раз в
   * секунду, и список перерисовывался бы с ним впустую.
   */
  const present = useSyncExternalStore(subscribe, () => gpuPresent(getState()));
  const noGpu = present === false || (present === undefined && settings.noGpu);
  const blocked = (widget: WidgetRow) => noGpu && widget.id === "gpu" && !widget.enabled;

  /*
   * Выключенные только что модули с программами: под списком им предлагают
   * удалить то, что больше никому не нужно. Живёт до закрытия окна, на диск
   * не попадает — решение разовое.
   */
  const [offered, setOffered] = useState<ModuleId[]>([]);
  // Отказ включения: окно не собралось, и Rust вернул флаг назад. Очередь
  // переходов держит Rust (номерами, в порядке нажатий — `modules::ticket`)
  const [error, setError] = useState<string | null>(null);

  const toggle = async (id: string, on: boolean) => {
    setError(null);
    pick(id);
    try {
      await setModule(id, on);
      const module = TOOL_MODULES.find((entry) => entry === id);
      if (module === undefined) return;
      setOffered((now) => (on ? now.filter((entry) => entry !== module) : [...new Set([...now, module])]));
    } catch (reason) {
      setError(typeof reason === "string" ? reason : t("common.failed"));
    }
  };

  const widgets = settings.widgets.filter((widget) => !widget.service);
  const groups: { title: Key; rows: WidgetRow[] }[] = [
    { title: "settings.widgets.enabled", rows: widgets.filter((widget) => widget.enabled) },
    { title: "settings.widgets.available", rows: widgets.filter((widget) => !widget.enabled) },
    { title: "settings.widgets.service", rows: settings.widgets.filter((widget) => widget.service) },
  ];
  const ordered = groups.flatMap((group) => group.rows);
  const current = ordered.find((widget) => widget.id === picked) ?? ordered[0];

  return (
    <div className="gallery">
      <div className="gallery__list">
        {settings.hiddenAll && <p className="note">{t("settings.widgets.allHidden")}</p>}
        {error && <p className="note note--error" role="alert">{error}</p>}
        {offered.length > 0 && (
          <ToolsOffer modules={offered} settings={settings} onClose={() => setOffered([])} />
        )}

        {groups.map(({ title, rows }) =>
          rows.length === 0 ? null : (
            <section key={title} className="gallery__group" aria-label={t(title)}>
              <h3 className="settings__subheading caps">{t(title)}</h3>
              {rows.map((widget) => (
                <Card
                  key={widget.id}
                  widget={widget}
                  selected={widget.id === current?.id}
                  blocked={blocked(widget)}
                  onPick={() => pick(widget.id)}
                  onOpen={() => navigate(widget.id)}
                  onToggle={(on) => void toggle(widget.id, on)}
                />
              ))}
            </section>
          ),
        )}
      </div>

      {current !== undefined && (
        <aside className="gallery__preview" aria-label={current.title}>
          {PREVIEWED.includes(current.id) && <LivePreview key={current.id} widget={current} settings={settings} />}
          <div className="gallery__caption">
            <span className="gallery__name">{current.title}</span>
            <p className="gallery__about">{t(about(current.id))}</p>
            <Button variant="outline" onClick={() => navigate(current.id)}>
              {t("settings.widgets.configure")}
            </Button>
          </div>
        </aside>
      )}
    </div>
  );
}

/**
 * Карточка виджета. Две кнопки рядом, а не одна в другой: вложенные кнопки
 * недопустимы, и клавиатура не дошла бы до внутренней.
 */
function Card({
  widget,
  selected,
  blocked,
  onPick,
  onOpen,
  onToggle,
}: {
  widget: WidgetRow;
  selected: boolean;
  blocked: boolean;
  onPick: () => void;
  onOpen: () => void;
  onToggle: (on: boolean) => void;
}) {
  const t = useText();
  const Icon = WIDGET_ICONS[widget.id];
  const card = useRef<HTMLDivElement>(null);

  // Карточка переехала в другую группу — выбранная остаётся на виду
  useLayoutEffect(() => {
    if (selected) card.current?.scrollIntoView({ block: "nearest" });
  }, [selected, widget.enabled]);

  const action = widget.enabled
    ? t("settings.widgets.disableOne", { title: widget.title })
    : blocked
      ? t("settings.widgets.noGpu")
      : t("settings.widgets.enableOne", { title: widget.title });

  return (
    <div ref={card} className="widget-card" data-selected={selected || undefined} data-enabled={widget.enabled || undefined}>
      <button
        type="button"
        className="widget-card__body"
        aria-pressed={selected}
        onClick={onPick}
        onDoubleClick={onOpen}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          onOpen();
        }}
      >
        <span className="widget-card__icon">{Icon !== undefined && <Icon />}</span>
        <span className="widget-card__text">
          <span className="widget-card__name" title={widget.title}>{widget.title}</span>
          <span className="widget-card__about">{t(about(widget.id))}</span>
        </span>
      </button>
      <button
        type="button"
        className="widget-card__action"
        aria-label={action}
        title={action}
        disabled={blocked}
        onClick={() => onToggle(!widget.enabled)}
      >
        {widget.enabled ? <CloseIcon /> : <CheckIcon />}
      </button>
    </div>
  );
}

/**
 * Живое превью: настоящая карточка виджета в рамке, на демоданных.
 *
 * Рамка — та же страница виджета с `?preview`; IPC Tauri там подменён
 * (`src/preview/install.ts`), и настоящего во вложенной рамке нет вовсе.
 * Отсюда в рамку уходят вид, язык и снимок настроек — всё, что виджет
 * спросил бы у Rust про себя самого, — обратно приходит высота у карточек,
 * которые встают по содержимому.
 *
 * Рамка размером с окно виджета при 100 % и уменьшается под ширину панели
 * целиком, как картинка; мышь и фокус в неё не попадают.
 */
function LivePreview({ widget, settings }: { widget: WidgetRow; settings: SettingsView }) {
  const t = useText();
  const look = useLook();
  const frame = useRef<HTMLIFrameElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const [measured, setMeasured] = useState<number | null>(null);
  const [room, setRoom] = useState(0);
  const [ready, setReady] = useState(false);

  const state: PreviewState = {
    look: {
      design: look.design,
      effectiveDesign: look.effectiveDesign,
      cardOpacity: look.cardOpacity,
      scale: look.scale,
      palette: look.palette,
      glass: look.glass,
      accentColor: look.accentColor,
      customColors: look.customColors,
      vivid: look.vivid,
    },
    language: settings.language ?? settings.systemLanguage,
    settings,
  };
  const latest = useRef(state);
  latest.current = state;
  // Рамка ещё не сказала, что готова: слать ей пока некому
  const listening = useRef(false);

  // Повторное письмо с тем же видом безвредно: `look.ts` в рамке одинаковый
  // вид отбрасывает сам, а язык рамка сравнивает по значению
  const post = () => {
    const target = frame.current?.contentWindow;
    if (target == null || !listening.current) return;
    const message: ToFrame = { type: TO_FRAME, state: latest.current };
    target.postMessage(message, window.location.origin);
  };
  const lookKey = JSON.stringify(state.look);

  useEffect(() => {
    const hear = (event: MessageEvent<ToHost>) => {
      if (event.origin !== window.location.origin || event.source !== frame.current?.contentWindow) return;
      if (event.data?.type === READY) {
        listening.current = true;
        post();
      }
      if (event.data?.type === HEIGHT && Number.isFinite(event.data.height)) setMeasured(event.data.height);
    };
    window.addEventListener("message", hear);
    return () => window.removeEventListener("message", hear);
  }, []);

  useEffect(() => {
    post();
  }, [lookKey, state.language, settings]);

  // Ширина панели: превью уменьшается под неё, но не растёт больше 100 %
  useEffect(() => {
    const element = box.current;
    if (element === null) return;
    const observer = new ResizeObserver(() => setRoom(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const [width, height] = [widget.preview[0], measured ?? widget.preview[1]];
  const fit = room > 0 ? Math.min(1, room / width) : 1;

  return (
    <div ref={box} className="gallery__stage">
      <div className="gallery__frame" style={{ width: width * fit, height: height * fit }} data-ready={ready || undefined}>
        <iframe
          ref={frame}
          title={t("settings.widgets.previewOf", { title: widget.title })}
          src={`?widget=${encodeURIComponent(widget.id)}&preview`}
          tabIndex={-1}
          aria-hidden="true"
          style={{ width, height, transform: `scale(${fit})` }}
          onLoad={() => setReady(true)}
        />
      </div>
    </div>
  );
}

/**
 * Предложение удалить программы выключенных модулей — ровно те, что скачаны и
 * не нужны ни одному включённому. ffmpeg общий у конвертера и загрузчика:
 * выключили один — ffmpeg остаётся другому, и предлагать его нечего.
 *
 * Отдельным компонентом, а не строкой списка: стор программ спрашивает Rust, а
 * тот по первому вопросу ищет ffmpeg в системе. Открытие списка виджетов
 * такого поиска не заслуживает — только выключение модуля с программами.
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
