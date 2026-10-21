import { useEffect, useRef, useState, type ComponentType } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { closeSettings, useSettings } from "../../lib/settings";
import { useLook } from "../../lib/look";
import { useWindowDrag } from "../../lib/drag";
import { useText, type Key } from "../../lib/i18n";
import {
  ChevronIcon,
  DataIcon,
  DocumentIcon,
  HeartIcon,
  PaletteIcon,
  PlusIcon,
  SlidersIcon,
  TilesIcon,
} from "../../components/Icons";
import { MAX_ACCOUNTS, useMail } from "../../lib/mail";
import { WIDGET_ICONS } from "../icons";
import { Navigate } from "./nav";
import { WidgetsSection } from "./WidgetsSection";
import { WidgetPage } from "./WidgetPage";
import { AppearanceSection, GeneralSection } from "./LookSection";
import { SourceSection } from "./SourceSection";
import { LicenseSection } from "./LicenseSection";
import { SupportSection } from "./SupportSection";
import { Hints } from "./Hints";
import "./settings.css";
import "./settings-reference.css";

/*
 * Окно настроек: шапка, колонка разделов слева, содержимое справа.
 *
 * Колонка устроена по образцу Themia (этап 52, слово пользователя): разделы со
 * значками, а под «Виджетами» — дерево включённых виджетов, у каждого свой
 * пункт. Новый раздел добавляется одной записью в списке ниже и одним файлом
 * рядом.
 *
 * Страница виджета — лист дерева, а не раздел: разделы стоят в колонке всегда,
 * а виджетов столько, сколько включено. Открывают её и из колонки, и
 * шестерёнкой в таблице, и пунктом «Настройки…» в меню самого виджета.
 */

type Section = {
  id: string;
  title: Key;
  Icon: ComponentType;
  Content: ComponentType;
};

const SECTIONS = [
  { id: "general", title: "settings.section.general", Icon: SlidersIcon, Content: GeneralSection },
  { id: "appearance", title: "settings.section.appearance", Icon: PaletteIcon, Content: AppearanceSection },
  { id: "widgets", title: "settings.section.widgets", Icon: TilesIcon, Content: WidgetsSection },
  { id: "source", title: "settings.section.source", Icon: DataIcon, Content: SourceSection },
  { id: "license", title: "settings.section.license", Icon: DocumentIcon, Content: LicenseSection },
  { id: "support", title: "settings.section.support", Icon: HeartIcon, Content: SupportSection },
] as const satisfies readonly Section[];

/** Раздел, под которым растёт дерево виджетов. */
const WIDGETS = "widgets";

/** Виджет, у которого в дереве второй уровень — его ящики (этап 53). */
const MAIL = "mail";

/**
 * Чью страницу показывать. Пусто — общие настройки.
 *
 * Только событием, без разбора адреса: окно собирается при старте и живёт до
 * выхода, поэтому `?for=` в нём было бы прочитано один раз за запуск и
 * навсегда осталось бы первым виджетом.
 *
 * Цель с приставкой `section:` — не страница, а раздел колонки: так карточка
 * напоминания о донатах открывает «Поддержать» (`SUPPORT_TARGET` в Rust).
 */
const SECTION = "section:";

function useTarget(onSection: (id: string) => void): [string, (id: string) => void] {
  const [id, setId] = useState("");
  const toSection = useRef(onSection);
  toSection.current = onSection;

  useEffect(() => {
    // Вне Tauri слушать некого, и молчаливый отказ тут правильнее исключения:
    // окно открывают только из приложения
    const stop = listen<string>("settings-target", (event) => {
      const payload = event.payload;
      if (payload.startsWith(SECTION)) {
        toSection.current(payload.slice(SECTION.length));
        setId("");
        return;
      }
      setId(payload);
    }).catch(() => () => {});

    return () => {
      void stop.then((off) => off());
    };
  }, []);

  return [id, setId];
}

/**
 * Показано ли окно. Спрятанное окно страницу не держит: закрытое на «Виджетах»
 * или странице процессора, оно иначе круглые сутки разбирало бы снимок
 * датчиков каждую секунду. Раздел и цель живут выше и переживают закрытие.
 *
 * Спрашиваем и слушаем разом; ответ на вопрос выбрасывается, если событие
 * успело раньше — оно свежее.
 */
function useShown(): boolean {
  const [shown, setShown] = useState(false);

  useEffect(() => {
    let heard = false;
    getCurrentWebviewWindow()
      .isVisible()
      .then((visible) => {
        if (!heard) setShown(visible);
      })
      .catch(() => {
        // Вне Tauri окно в браузере видно всегда — замеры раскладки там же
        if (!heard) setShown(true);
      });

    const stop = listen<boolean>("settings-shown", (event) => {
      heard = true;
      setShown(event.payload);
    }).catch(() => () => {});

    return () => {
      void stop.then((off) => off());
    };
  }, []);

  return shown;
}

export function SettingsWindow() {
  // Блокировку положения окно настроек не слушает: в нём стоит её выключатель
  useWindowDrag(true);
  const t = useText();
  const settings = useSettings();
  const reference = useLook().effectiveDesign === "reference";

  const shown = useShown();

  const [active, setActive] = useState<string>(SECTIONS[0].id);
  const [target, setTarget] = useTarget((id) => {
    if (SECTIONS.some((item) => item.id === id)) setActive(id);
  });
  /**
   * Свёрнутые ветки: «Виджеты» и «Почта». Живут до выхода: окно настроек
   * собирается один раз на запуск, а на диск такое не пишем — `localStorage`
   * в проекте не заводится вовсе. По умолчанию развёрнуто всё.
   */
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set());
  const fold = (id: string) =>
    setFolded((was) => {
      const next = new Set(was);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const section = SECTIONS.find((item) => item.id === active) ?? SECTIONS[0];
  const { Content } = section;

  // Цель бывает составной: `mail/<ключ ящика>` — страница ящика под почтой
  const [targetId, page] = target.includes("/") ? (target.split("/", 2) as [string, string]) : [target, undefined];

  // Дерево — только включённые модули: выключенный не грузится, и настраивать
  // его зовут шестерёнкой в таблице
  const leaves = settings.widgets.filter((widget) => widget.enabled);
  const onLeaf = targetId !== "" && leaves.some((widget) => widget.id === targetId);
  const boxes = useMail()?.accounts ?? [];

  // Подсвечен лист, если страница его; страница выключенного модуля, которого
  // в дереве нет, подсвечивает «Виджеты» — иначе колонка не показала бы ничего
  const current = target === "" ? section.id : onLeaf ? "" : WIDGETS;

  const go = (id: string) => {
    setActive(id);
    setTarget("");
  };

  // Esc закрывает — привычка любого окна настроек. Но не из поля ввода: там
  // Esc первым делом отпускает поле. Закрытое окно размонтирует страницу, и
  // недосохранённый пароль ящика, секрет 2FA или запрос города пропали бы
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const field = event.target;
      if (field instanceof HTMLElement && (field.isContentEditable || field.matches("input:not([type=range]):not([type=checkbox]), textarea, select"))) {
        field.blur();
        return;
      }
      closeSettings();
    };

    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, []);

  return (
    <div className={reference ? "settings settings--reference" : "settings"}>
      <header className="settings__head">
        <span className="settings__title">{t("window.settings")}</span>
        <button
          type="button"
          className="settings__close"
          aria-label={t("settings.close")}
          title={t("settings.closeEsc")}
          onClick={closeSettings}
        >
          <svg viewBox="0 0 14 14" aria-hidden="true">
            <path d="M3 3 11 11 M11 3 3 11" />
          </svg>
        </button>
      </header>

      <div className="settings__body">
        <nav className="settings__nav">
          {SECTIONS.map(({ id, title, Icon }) => (
            <div key={id} className="settings__branch" data-section={reference ? id : undefined}>
              <div className="settings__row">
                <button type="button" className="settings__tab" aria-current={id === current} onClick={() => go(id)}>
                  <span className="settings__icon">
                    <Icon />
                  </span>
                  <span className="settings__label">{t(title)}</span>
                </button>
                {id === WIDGETS && leaves.length > 0 && (
                  <button
                    type="button"
                    className="settings__fold"
                    aria-expanded={!folded.has(WIDGETS)}
                    aria-label={t(folded.has(WIDGETS) ? "settings.nav.expand" : "settings.nav.collapse")}
                    onClick={() => fold(WIDGETS)}
                  >
                    <ChevronIcon />
                  </button>
                )}
              </div>

              {id === WIDGETS && !folded.has(WIDGETS) && leaves.length > 0 && (
                <div className="settings__leaves" role="group" aria-label={t(title)}>
                  {leaves.map((widget) => {
                    const LeafIcon = WIDGET_ICONS[widget.id];
                    const leaf = (
                      <button
                        key={widget.id}
                        type="button"
                        className="settings__tab settings__tab--leaf"
                        aria-current={target === widget.id}
                        title={widget.title}
                        onClick={() => setTarget(widget.id)}
                      >
                        <span className="settings__icon">{LeafIcon !== undefined && <LeafIcon />}</span>
                        <span className="settings__label">{widget.title}</span>
                      </button>
                    );
                    if (widget.id !== MAIL) return leaf;

                    /*
                     * Почта — ветка второго уровня: ящики по адресу и «Добавить
                     * ящик». Своя стрелка, своё сворачивание; развёрнута по
                     * умолчанию, как и «Виджеты».
                     */
                    const open = !folded.has(MAIL);
                    return (
                      <div key={widget.id} className="settings__branch">
                        <div className="settings__row">
                          {leaf}
                          <button
                            type="button"
                            className="settings__fold"
                            aria-expanded={open}
                            aria-label={t(open ? "settings.nav.collapseBranch" : "settings.nav.expandBranch", { name: widget.title })}
                            onClick={() => fold(MAIL)}
                          >
                            <ChevronIcon />
                          </button>
                        </div>
                        {open && (
                          <div className="settings__leaves" role="group" aria-label={widget.title}>
                            {boxes.map((box) => (
                              <button
                                key={box.id}
                                type="button"
                                className="settings__tab settings__tab--sub"
                                aria-current={target === `${MAIL}/${box.id}`}
                                title={box.address}
                                onClick={() => setTarget(`${MAIL}/${box.id}`)}
                              >
                                <span className="settings__label">{box.address || t("mail.settings.newBox")}</span>
                              </button>
                            ))}
                            <button
                              type="button"
                              className="settings__tab settings__tab--sub"
                              aria-current={target === `${MAIL}/new`}
                              disabled={boxes.length >= MAX_ACCOUNTS}
                              title={boxes.length >= MAX_ACCOUNTS ? t("mail.settings.limit", { limit: MAX_ACCOUNTS }) : undefined}
                              onClick={() => setTarget(`${MAIL}/new`)}
                            >
                              <span className="settings__icon">
                                <PlusIcon />
                              </span>
                              <span className="settings__label">{t("mail.settings.add")}</span>
                            </button>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          ))}
        </nav>

        <section className="settings__panel">
          <Navigate.Provider value={setTarget}>
            {!shown ? null : target === "" ? (
              <>
                <h2 className="settings__heading caps">{t(section.title)}</h2>
                <Content />
              </>
            ) : (
              <WidgetPage id={targetId} page={page} />
            )}
          </Navigate.Provider>
        </section>
      </div>

      <Hints />
    </div>
  );
}
