import { StrictMode, Suspense, useEffect, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { resolveWidget, WIDGETS } from "./widgets/registry";
import { initLook } from "./lib/look";
import { initText, useText, useTextFailed, useTextReady, type Key } from "./lib/i18n";
import { useWidgetMenu } from "./lib/gestures";
import { Guard, TextFailed } from "./components/Guard";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/reference.css";
import "./styles/design.css";

// Плотность фона и кегль общие на все окна, поэтому подхватываются до
// отрисовки виджета. Язык — по той же причине
initLook();
initText();

// Каждое окно показывает ровно один виджет, его id приходит в query-параметре
const requestedId = new URLSearchParams(window.location.search).get("widget") ?? "debug";
const widget = resolveWidget(requestedId);

function UnknownWidget({ id }: { id: string }) {
  const t = useText();
  return (
    <div className="fallback">
      <p>{t("app.unknownWidget", { id })}</p>
      <p>{t("app.availableWidgets", { list: Object.keys(WIDGETS).join(", ") })}</p>
    </div>
  );
}

/** Ключ названия окна: у виджета — его строка в каталоге, у служебных окон — своя. */
const OWN_TITLES: Record<string, Key> = {
  settings: "window.settings",
  "support-reminder": "window.supportReminder",
};
const TITLE_KEY = OWN_TITLES[requestedId] ?? (`widget.${requestedId}` as Key);

/**
 * Оболочка окна: одна на все окна, включая настройки и витрину.
 *
 * Держит то, что нужно каждому окну и ни одному виджету в отдельности:
 * запрет меню WebView2 и своё меню по правой кнопке. Виджет, забывший позвать
 * этот хук у себя, вернул бы в окно «Обновить / Сохранить как / Печать», —
 * поэтому вызов стоит здесь, а не в карточках.
 *
 * Окно настроек меню виджета не получает: `open_widget_menu` не находит его в
 * реестре и молча возвращается.
 */
function Shell({ children }: { children: ReactNode }) {
  useWidgetMenu();
  const t = useText();
  const ready = useTextReady();
  const failed = useTextFailed();

  useEffect(() => {
    if (ready && widget) document.title = t(TITLE_KEY);
  }, [ready, t]);

  // ВАЖНО: **пока словарь не приехал, окно не рисует ничего.** Иначе на
  // первом кадре стояли бы ключи вместо слов. Окна собираются спрятанными,
  // и пустой миг никто не увидит. Не приехал вовсе — кнопка повтора, а не
  // вечная пустота
  if (failed) return <TextFailed />;
  if (!ready) return null;
  return <Guard>{children}</Guard>;
}

const container = document.getElementById("root");
if (!container) throw new Error("Не найден корневой элемент #root");

const Widget = widget?.component;

// Код виджета грузится лениво (`registry.ts`); пока он едет, окно пустое —
// как и до словаря: окна собираются спрятанными, пустого мига никто не видит
createRoot(container).render(
  <StrictMode>
    <Shell>
      {Widget ? (
        <Suspense fallback={null}>
          <Widget />
        </Suspense>
      ) : (
        <UnknownWidget id={requestedId} />
      )}
    </Shell>
  </StrictMode>,
);
