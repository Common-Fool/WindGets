import { Fragment, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import iconWidgetMosaic from "../../assets/app-icons/widget-mosaic.png";
import iconMonoMosaic from "../../assets/app-icons/mono-mosaic.png";
import { Slider } from "../../components/Slider";
import { ChevronIcon } from "../../components/Icons";
import {
  DEFAULT_CUSTOM_COLORS,
  MAX_SCALE,
  MAX_VIVID,
  MIN_OPACITY,
  MIN_SCALE,
  MIN_VIVID,
  resetCustomColors,
  setAccentColor,
  setCustomColor,
  setGlass,
  setPalette,
  setDesign,
  useLook,
} from "../../lib/look";
import { setAppIcon, useSettings, type ChoiceRow } from "../../lib/settings";
import { WindowsFields } from "./WindowsSection";
import { UpdateField } from "./UpdateField";
import { setLanguage, useText, type Key, type Language } from "../../lib/i18n";
import {
  isReferenceTheme,
  REFERENCE_COLOR_ROLES,
  REFERENCE_THEMES,
} from "../../lib/referenceThemes";

/** Строка под именем режима. У тем описания свои — в `referenceThemes.ts`. */
const MODE_DESCRIPTIONS: Record<string, Key> = {
  monochrome: "look.mode.monochrome",
  single: "look.mode.single",
  custom: "look.mode.custom",
};

/**
 * Темы, которые стоят в списке первыми, — порядок пользователя (этап 53).
 *
 * Порядок живёт здесь, а не в `settings::PALETTES`: первая запись там — тема
 * по умолчанию, и перестановка ради списка сменила бы её молча. Дальше идут
 * прочие темы в порядке Rust, последними — три режима.
 */
const FIRST_THEMES = ["reference-a", "reference-b", "reference-c"];

/** Место темы в списке: первые три, прочие темы, режимы. */
function rank(id: string): number {
  if (id === "gpt") return -1;
  const first = FIRST_THEMES.indexOf(id);
  if (first >= 0) return first;
  return isReferenceTheme(id) ? FIRST_THEMES.length : FIRST_THEMES.length + 1;
}

/**
 * Четыре базы режимов и то, где каждая из них повторяется. Порядок тот же, что
 * у полей «Своей палитры».
 */
const COLOR_ROLES = [
  { id: "accent", title: "look.base.accent", usage: "look.base.accentUsage", token: "--color-accent" },
  { id: "positive", title: "look.base.positive", usage: "look.base.positiveUsage", token: "--metric-cpu" },
  { id: "caution", title: "look.base.caution", usage: "look.base.cautionUsage", token: "--metric-ram" },
  { id: "negative", title: "look.base.negative", usage: "look.base.negativeUsage", token: "--metric-net-up" },
] as const satisfies readonly { id: string; title: Key; usage: Key; token: string }[];

const APP_ICON_CHOICES = [
  { id: "widget-mosaic", image: iconWidgetMosaic, title: "look.appIcon.widgetMosaic" },
  { id: "mono-mosaic", image: iconMonoMosaic, title: "look.appIcon.monoMosaic" },
] as const satisfies readonly { id: string; image: string; title: Key }[];

/** Готовые образцы «Одного акцента»; любой другой — полем HEX или пипеткой. */
const ACCENT_CHOICES: { color: string; title: Key }[] = [
  { color: "#4CA5FF", title: "look.color.blue" },
  { color: "#76AAFF", title: "look.color.sky" },
  { color: "#65D2CE", title: "look.color.turquoise" },
  { color: "#69D6AC", title: "look.color.mint" },
  { color: "#E6BD68", title: "look.color.gold" },
  { color: "#F28D80", title: "look.color.coral" },
  { color: "#F080AD", title: "look.color.pink" },
  { color: "#EEEEEE", title: "look.color.white" },
];

function swatchStyle(token: string): CSSProperties {
  return { background: `var(${token})` };
}

/**
 * Общие: язык и масштаб — то, что меняет всё приложение разом, а не его цвета.
 *
 * Отдельным разделом от оформления по слову пользователя (этап 52, по образцу
 * Themia): «Вид» держал и язык, и восемь тем, и до масштаба в самом низу было
 * не долистать.
 *
 * Ползунок показывает результат сразу во всех окнах, а запоминает его один
 * раз — по отпусканию: запись на диск на каждый пиксель стоила бы дороже самой
 * отрисовки.
 */
export function GeneralSection() {
  const look = useLook();
  const t = useText();
  const scale = Math.round(look.scale * 100);

  return (
    <div className="rows">
      <LanguageChoice />

      <div className="field" data-hint={t("look.scaleNote")}>
        <div className="field__head">
          <span className="field__name">{t("look.scale")}</span>
          <span className="field__value tabular">{scale} %</span>
        </div>
        <Slider
          value={scale}
          min={MIN_SCALE * 100}
          max={MAX_SCALE * 100}
          step={5}
          onChange={(next) => look.preview({ scale: next / 100 })}
          onCommit={look.commit}
        />
        <div className="field__scale"><span>{Math.round(MIN_SCALE * 100)} %</span><span>{Math.round(MAX_SCALE * 100)} %</span></div>
      </div>

      {/* Бывший раздел «Окна» (7 октября, слово пользователя): четыре
          рубильника и сброс не тянули на свой пункт в колонке */}
      <hr className="divider" />
      <h3 className="settings__subheading caps">{t("settings.section.windows")}</h3>
      <WindowsFields />

      <hr className="divider" />
      <h3 className="settings__subheading caps">{t("update.title")}</h3>
      <UpdateField />
    </div>
  );
}

/**
 * Оформление: тема, её цвета, стекло и плотность фона.
 *
 * Ползунки — как у масштаба: показывают сразу, запоминают по отпусканию. Тему
 * и стекло выбирают нажатием, и пишутся они сразу.
 */
export function AppearanceSection() {
  const look = useLook();
  const t = useText();
  const [resetVersion, setResetVersion] = useState(0);
  const [designPending, setDesignPending] = useState(false);
  const [designError, setDesignError] = useState(false);
  async function chooseDesign(design: "app" | "reference") {
    setDesignPending(true); setDesignError(false);
    try { await setDesign(design); }
    catch { setDesignError(true); }
    finally { setDesignPending(false); }
  }
  const opacity = Math.round(look.cardOpacity * 100);
  const vivid = Math.round(look.vivid * 100);

  /* Один набор полей с постоянными ключами. При перестановке GPT/Claude
   * React сохраняет формы и черновики, а Tab следует видимому порядку. */
  const fields: { id: string; content: ReactNode }[] = [
    { id: "design", content: (
      <div className="field" data-look-field="design" data-hint={t("look.designNote")}>
        <div className="field__head"><span className="field__name">{t(look.effectiveDesign === "reference" ? "settings.design.global" : "look.design")}</span></div>
        <div className="choices" role="group" aria-label={t("look.design")}>
          {(["reference", "app"] as const).map((design) => (
            <button key={design} type="button" className="choice" aria-pressed={look.design === design}
              disabled={designPending} onClick={() => void chooseDesign(design)}>{t(design === "reference" ? "design.gpt" : "design.claude")}</button>
          ))}
        </div>
      </div>
    ) },
    { id: "design-error", content: designError && <p className="note" role="alert" data-look-field="design-error">{t("look.designError")}</p> },
    { id: "scope", content: <p className="note" data-look-field="scope">{t("look.designScope")}</p> },
    { id: "palette", content: <Palette /> },
    { id: "single", content: look.palette === "single" && (
        <div className="field" data-look-field="single">
          <ColorField
            label={t("look.accentColor")}
            usage={t("look.accentColorUsage")}
            value={look.accentColor}
            key={look.effectiveDesign}
            onCommit={(color) => setAccentColor(color, look.effectiveDesign)}
            palette
          />
        </div>
      ) },
    { id: "custom", content: look.palette === "custom" && (
        <div className="field" data-look-field="custom">
          <div className="field__head">
            <span className="field__name">{t("look.customColors")}</span>
            <button
              className="choice color-reset"
              type="button"
              onClick={() => {
                resetCustomColors(look.effectiveDesign);
                setResetVersion((version) => version + 1);
              }}
            >
              {t("look.resetColors")}
            </button>
          </div>
          <div className="color-fields">
            {COLOR_ROLES.map((role) => (
              <ColorField
                key={`${look.effectiveDesign}/${role.id}`}
                label={t(role.title)}
                usage={t(role.usage)}
                value={look.customColors[role.id] ?? DEFAULT_CUSTOM_COLORS[role.id]!}
                onCommit={(color) => setCustomColor(role.id, color, look.effectiveDesign)}
                resetVersion={resetVersion}
                resetValue={DEFAULT_CUSTOM_COLORS[role.id]}
              />
            ))}
          </div>
          <p className="note">{t("look.whiteText")}</p>
        </div>
      ) },
    { id: "legend", content: <ColorLegend /> },
    { id: "vivid", content: (
      <div className="field" data-look-field="vivid" data-hint={t("look.vividNote")}>
        <div className="field__head">
          <span className="field__name">{t("look.vivid")}</span>
          <span className="field__value tabular">{vivid} %</span>
        </div>
        <Slider
          value={vivid}
          min={MIN_VIVID * 100}
          max={MAX_VIVID * 100}
          step={5}
          onChange={(next) => look.preview({ vivid: next / 100 })}
          onCommit={look.commit}
        />
        <div className="field__scale"><span>{t("look.vividLow")}</span><span>{t("look.vividHigh")}</span></div>
      </div>
    ) },
    { id: "glass", content: <GlassPalette /> },
    { id: "opacity", content: (
      <div className="field" data-look-field="opacity" data-hint={t("look.opacityNote")}>
        <div className="field__head">
          <span className="field__name">{t("look.opacity")}</span>
          <span className="field__value tabular">{opacity} %</span>
        </div>
        <Slider
          value={opacity}
          min={MIN_OPACITY * 100}
          max={100}
          onChange={(next) => look.preview({ cardOpacity: next / 100 })}
          onCommit={look.commit}
        />
        <div className="field__scale"><span>{t("look.opacityLow")}</span><span>{t("look.opacityHigh")}</span></div>
      </div>
    ) },
    { id: "app-icon", content: <AppIconPicker /> },
  ];
  if (look.effectiveDesign === "reference") {
    const order = ["design", "design-error", "scope", "palette", "single", "custom", "glass", "vivid", "opacity", "legend", "app-icon"];
    fields.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
  }

  return (
    <div className="rows" data-settings-page={look.effectiveDesign === "reference" ? "appearance" : undefined}>
      {fields.map(({ id, content }) => <Fragment key={id}>{content}</Fragment>)}
    </div>
  );
}

function AppIconPicker() {
  const settings = useSettings();
  const t = useText();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);

  async function choose(id: string) {
    if (pending || id === settings.appIcon) return;
    setPending(true);
    setError(false);
    try {
      await setAppIcon(id);
    } catch {
      setError(true);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="field" data-look-field="app-icon">
      <div className="field__head">
        <span className="field__name">{t("look.appIcon")}</span>
      </div>
      <div className="app-icon-grid" role="group" aria-label={t("look.appIcon")}>
        {APP_ICON_CHOICES.map((option) => {
          const selected = settings.appIcon === option.id;
          return (
            <button
              key={option.id}
              type="button"
              className="app-icon-choice"
              aria-label={t(option.title)}
              aria-pressed={selected}
              disabled={pending}
              onClick={() => void choose(option.id)}
            >
              <img src={option.image} alt="" />
              <span className="app-icon-choice__name">{t(option.title)}</span>
              {selected && <span className="app-icon-choice__check" aria-hidden="true">✓</span>}
            </button>
          );
        })}
      </div>
      {error && <p className="note note--error" role="alert">{t("look.appIconError")}</p>}
    </div>
  );
}

/**
 * Выбор темы: восемь тем и три режима — одним постоянным списком тонких строк.
 *
 * Карточками по три в ряд они занимали полтора экрана (этап 52, слово
 * пользователя): описание и легенда на каждой карточке читались один раз, а
 * листать мимо них приходилось всякий раз. Описание теперь всплывает
 * подсказкой у строки, образец остался. Деления на «Темы» и «Режимы» нет с
 * этапа 53: выбирают из одного списка, а режимы просто стоят в его конце.
 *
 * Своей у виджета тема не бывает и не будет: разойдись она по виджетам, за
 * метрикой не осталось бы закреплено ничего.
 */
function Palette() {
  const settings = useSettings();
  const look = useLook();
  const t = useText();
  // Сортировка устойчивая: внутри своей ступени темы идут в порядке Rust
  const palettes = [...settings.palettes].filter((item) => item.id !== "gpt" || look.effectiveDesign === "reference").sort((a, b) => rank(a.id) - rank(b.id));

  return (
    <div className="field" data-look-field="palette">
      {/* Общая подсказка — на заголовке, у каждой строки своя: описание темы */}
      <div className="field__head" data-hint={t("look.paletteNote")}>
        <span className="field__name" id="palette-title">{t(look.effectiveDesign === "reference" ? "settings.palette.data" : "look.palette")}</span>
      </div>
      <div className="theme-list" role="group" aria-labelledby="palette-title">
        {palettes.map((item) => <ThemeChoice key={item.id} item={item} selected={look.palette} />)}
      </div>
    </div>
  );
}

/**
 * Строка темы: образец слева, имя, галочка у выбранной.
 *
 * Образец — не кнопка с именем, а четыре столбика цветами процессора,
 * видеокарты, памяти и отдачи: одно имя не дало бы понять, насколько тема тихая
 * или тёплая. В режимах это ровно четыре базы, в темах — их назначения, и пары
 * процессор с памятью и приём с отдачей различаются в каждой теме
 * (`pairs_differ_in_every_theme`). Полосы акцента интерфейса над столбиками,
 * как у карточки этапа 40, нет: в тонкой строке она читалась непонятной чертой
 * (этап 53, слово пользователя). Описание — подсказкой.
 *
 * Цвета внутри пересчитывает `data-theme-preview`: токены берут базы этой
 * строки, поэтому все образцы видны разом, каждый своими цветами.
 */
function ThemeChoice({ item, selected }: { item: ChoiceRow; selected: string }) {
  const t = useText();
  const description = REFERENCE_THEMES[item.id]?.description ?? MODE_DESCRIPTIONS[item.id];
  return (
    <button
      type="button"
      className="theme-row"
      data-hint={description === undefined ? undefined : t(description)}
      aria-pressed={item.id === selected}
      onClick={() => setPalette(item.id)}
    >
      <span className="theme-row__preview" data-theme-preview={item.id} aria-hidden>
        <span className="theme-row__bar theme-row__bar--cpu" />
        <span className="theme-row__bar theme-row__bar--gpu" />
        <span className="theme-row__bar theme-row__bar--ram" />
        <span className="theme-row__bar theme-row__bar--up" />
      </span>
      <span className="theme-row__name">{item.title}</span>
      <span className="theme-row__mark" aria-hidden>✓</span>
    </button>
  );
}

/** Где какой цвет: десять назначений у темы, четыре базы у режима. */
function ColorLegend() {
  const look = useLook();
  const t = useText();
  const theme = isReferenceTheme(look.palette);
  const neutral = look.palette === "single" || look.palette === "monochrome";
  const roles = REFERENCE_COLOR_ROLES;
  return (
    <div className="field" data-look-field="legend">
      <div className="field__head"><span className="field__name">{t("look.legend")}</span></div>
      <div className={theme ? "color-legend color-legend--reference" : "color-legend"}>
        {roles.map((role) => (
          <div key={role.id}>
            <i style={swatchStyle(role.token)} aria-hidden />
            <span>{t(role.usage)}</span>
          </div>
        ))}
      </div>
      <p className="note">
        {t(theme ? "look.legendTheme" : neutral ? "look.legendNeutral" : "look.legendBases")}{" "}
        {t("look.legendWhite")}
      </p>
    </div>
  );
}

/**
 * Язык интерфейса: раскрывающийся список, первым — «как в Windows».
 *
 * Списком, а не шестью кнопками (этап 52, слово пользователя): язык выбирают
 * раз в жизни, и ряд кнопок занимал место, которое нужнее масштабу. Список
 * родной, `<select>`: клавиатура, колесо и раскрытие у него уже есть, и
 * раскрытый он рисуется поверх окна, а не внутри его прокрутки.
 *
 * Языки подписаны самоназваниями и не переводятся никогда: человек, случайно
 * включивший китайский, найдёт «Русский», не читая по-китайски. Каждому пункту
 * свой `lang` — по нему WebView2 берёт шрифт с иероглифами для «中文».
 */
function LanguageChoice() {
  const settings = useSettings();
  const t = useText();
  const system = settings.languages.find((item) => item.id === settings.systemLanguage);
  const title = t("look.language.title");

  return (
    <div className="field">
      <div className="field__head field__head--center">
        <label className="field__name" htmlFor="language">{title}</label>
        <span className="select">
          <select
            id="language"
            className="select__control"
            value={settings.language ?? ""}
            onChange={(event) => setLanguage(event.target.value === "" ? null : (event.target.value as Language))}
          >
            <option value="">
              {t("look.language.system", { language: system?.title ?? settings.systemLanguage })}
            </option>
            {settings.languages.map((item) => (
              <option key={item.id} value={item.id} lang={item.id === "zh" ? "zh-CN" : item.id}>
                {item.title}
              </option>
            ))}
          </select>
          <span className="select__chevron" aria-hidden>
            <ChevronIcon />
          </span>
        </span>
      </div>
    </div>
  );
}

/**
 * Стекло: шесть основ, от темы не зависят и при её смене остаются.
 *
 * Одной строкой образцов с именем внутри, а не карточками по три в ряд (этап
 * 52, слово пользователя): образец и есть цвет, подпись под ним повторяла его
 * же вторым этажом.
 */
function GlassPalette() {
  const settings = useSettings();
  const look = useLook();
  const t = useText();
  return (
    <div className="field" data-look-field="glass">
      <div className="field__head" data-hint={t("look.glassNote")}>
        <span className="field__name">{t("look.glass")}</span>
      </div>
      <div className="glass-choices" role="group" aria-label={t("look.glass")}>
        {settings.glasses.map((item) => (
          <button
            key={item.id}
            type="button"
            className="glass-choice"
            data-glass-preview={item.id}
            aria-pressed={item.id === look.glass}
            title={item.title}
            onClick={() => setGlass(item.id)}
          >
            <span className="glass-choice__name">{item.title}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function normalizeHex(value: string): string | null {
  const hex = value.trim().replace(/^#/, "");
  return /^[\da-f]{6}$/i.test(hex) ? `#${hex.toUpperCase()}` : null;
}

/**
 * Цвет полем HEX и пипеткой.
 *
 * Черновик держится локально и не откатывается от запоздавших ответов на
 * предыдущие правки: пипетку тянут, и событие о прошлом цвете, пришедшее
 * посреди движения, вернуло бы поле назад. Правки уходят не чаще раза в сто
 * двадцать миллисекунд и досылаются по уходу из поля.
 */
function ColorField({
  label, usage, value, onCommit, palette = false, resetVersion = 0, resetValue = value,
}: {
  label: string;
  usage: string;
  value: string;
  onCommit: (color: string) => void;
  palette?: boolean;
  resetVersion?: number;
  resetValue?: string;
}) {
  const t = useText();
  const [draft, setDraft] = useState(value.toUpperCase());
  const draftRef = useRef(draft);
  const pending = useRef<string | null>(null);
  const scheduled = useRef<string | null>(null);
  const lastSent = useRef(value.toUpperCase());
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const focused = useRef(false);
  const commitRef = useRef(onCommit);
  const resetSeen = useRef(resetVersion);
  commitRef.current = onCommit;

  function flush() {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
    if (scheduled.current !== null) {
      const color = scheduled.current;
      scheduled.current = null;
      lastSent.current = color;
      commitRef.current(color);
    }
  }

  useEffect(() => {
    const next = value.toUpperCase();
    if (pending.current !== null && pending.current !== next) return;
    if (pending.current === next) pending.current = null;
    lastSent.current = next;
    if (!focused.current) {
      draftRef.current = next;
      setDraft(next);
    }
  }, [value]);

  useEffect(() => {
    if (resetSeen.current === resetVersion) return;
    resetSeen.current = resetVersion;
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
    scheduled.current = null;
    pending.current = resetValue.toUpperCase();
    draftRef.current = pending.current;
    lastSent.current = pending.current;
    setDraft(pending.current);
  }, [resetVersion, resetValue]);

  useEffect(() => () => {
    if (timer.current !== null) clearTimeout(timer.current);
    if (scheduled.current !== null) commitRef.current(scheduled.current);
  }, []);

  function change(next: string) {
    draftRef.current = next;
    setDraft(next);
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
    scheduled.current = null;
    const color = normalizeHex(next);
    if (color === null) return;
    pending.current = color;
    scheduled.current = color;
    timer.current = setTimeout(flush, 120);
  }

  function finish() {
    focused.current = false;
    const color = normalizeHex(draftRef.current);
    const next = color ?? pending.current ?? value.toUpperCase();
    draftRef.current = next;
    setDraft(next);
    if (next !== lastSent.current) scheduled.current = next;
    flush();
  }

  const color = normalizeHex(draft) ?? pending.current ?? value;
  return (
    <div className="color-field">
      <div className="color-field__row">
        <div className="color-field__label"><span>{label}</span><span className="note">{usage}</span></div>
        <div className="color-field__inputs">
          <input
            type="color"
            className="color-field__picker"
            aria-label={t("look.pickColor", { label })}
            value={color}
            onFocus={() => { focused.current = true; }}
            onChange={(event) => change(event.target.value.toUpperCase())}
            onBlur={finish}
          />
          <input
            type="text"
            className="color-field__hex"
            aria-label={`${label}: HEX`}
            value={draft}
            maxLength={7}
            spellCheck={false}
            onFocus={() => { focused.current = true; }}
            onChange={(event) => change(event.target.value)}
            onBlur={finish}
            onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
          />
        </div>
      </div>
      {palette && (
        <div className="accent-choices" role="group" aria-label={t("look.accentPalette")}>
          {ACCENT_CHOICES.map((item) => (
            <button
              key={item.color}
              type="button"
              className="accent-choice"
              title={t(item.title)}
              aria-label={t(item.title)}
              aria-pressed={color.toUpperCase() === item.color}
              style={{ "--choice-color": item.color } as CSSProperties}
              onClick={() => { change(item.color); flush(); }}
            ><span aria-hidden /></button>
          ))}
        </div>
      )}
    </div>
  );
}
