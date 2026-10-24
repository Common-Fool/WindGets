import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { useWidgetCover, useWidgetLayout, useWidgetSize, useWidgetView } from "../../lib/view";
import { usePresetWindowGrowth } from "../../lib/grow";
import { useWindowResized } from "../../lib/resized";
import { useLook, useScale } from "../../lib/look";
import { GlassCard } from "../../components/GlassCard";
import {
  connect,
  control,
  failureOf,
  openSpotify,
  useSpotify,
  type Action,
  type Place,
} from "../../lib/spotify";
import { hasVolume, LAYOUT, PlayerPanel } from "./PlayerPanel";
import { CLAUDE_SIZES, REFERENCE_SIZES, referenceSlack } from "./ReferencePlayer";
import { PlaylistPanel } from "./PlaylistPanel";
import { SetupPanel } from "./SetupPanel";
import { GlobeIcon, PlaylistIcon, SpotifyMark } from "./icons";
import "./spotify.css";
import "./spotify-reference.css";
import { useText } from "../../lib/i18n";

/*
 * Список плейлистов сидит в том же окне, поэтому окно под него и растёт.
 * Держать его раскрытым всегда — значит отдать половину высоты пустоте.
 *
 * Число при кегле 100 %, дальше множится на геометрию. Точной меркой оно и не
 * было: плейлистов до полусотни, и остальное берёт на себя прокрутка списка.
 */
const PLAYLISTS_HEIGHT = 232;

/**
 * Сколько держится строка отказа.
 *
 * Гаснуть она обязана сама: прежде строка висела до следующего нажатия, и
 * «нет активного устройства» краснело под играющим треком часами — команда
 * не прошла минуту назад, а виджет выглядел сломанным.
 */
const FAILURE_FOR = 8000;


/** Отказ, который сам исчезает. */
function useFailure(): [string | null, (reason: string | null) => void] {
  const [failure, setFailure] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const show = useCallback((reason: string | null) => {
    window.clearTimeout(timer.current);
    setFailure(reason);

    if (reason !== null) {
      timer.current = window.setTimeout(() => setFailure(null), FAILURE_FOR);
    }
  }, []);

  return [failure, show];
}

export function SpotifyWidget() {
  const t = useText();

  // Раскладку выбирают из меню или из настроек виджета, щелчком по карточке
  // она не листается: раскладки разной формы, и случайный щелчок мимо кнопки
  // двигал бы соседей и терял выбранный размер
  const size = useWidgetSize();
  const widgetView = useWidgetView();
  const layout = useWidgetLayout();
  const { design, effectiveDesign } = useLook();
  const reference = effectiveDesign === "reference";
  const { view, at } = useSpotify();
  const [failure, setFailure] = useFailure();
  const [connecting, setConnecting] = useState(false);
  const [playlistsOpen, setPlaylistsOpen] = useState(false);

  // Размер поменяли — список плейлистов сворачиваем вместе с ним
  useWindowResized(() => setPlaylistsOpen(false));
  const growth = usePresetWindowGrowth(widgetView, size, design, layout);
  const geometry = useScale();

  const connected = view.status === "idle" || view.status === "active";

  // Отключились или потеряли связь — список плейлистов показывать не на чем
  useEffect(() => {
    if (connected || !playlistsOpen) return;
    setPlaylistsOpen(false);
    growth.close();
  }, [connected, playlistsOpen, growth]);

  /*
   * ВАЖНО: **слой запуска всплывает только на «играть»**, когда Спотифай
   * нигде не открыт, — просьба пользователя. Прежде он стоял поверх блёклого
   * пульта весь простой, и пульт под ним был виден, но не нажимался.
   *
   * Уходит слой по нажатию значка, по щелчку мимо значков и когда мышь
   * покидает окно: вопрос задан тому, кто сейчас у виджета, и без него
   * висеть незачем. Запуск ничего не включает — Спотифай только появляется
   * среди устройств, и «играть» жмут ещё раз.
   *
   * Исключение одно: пульт, который не видел ни одного трека. «Играть» там
   * нет вовсе, и слой стоит постоянно — с подсказкой из Rust.
   */
  const blank = view.status === "idle" && view.last === null ? t("spotify.idleHint") : null;
  const [asking, setAsking] = useState(false);

  // Сменилось состояние пульта — например, заиграло: спрашивать больше не о чем
  useEffect(() => setAsking(false), [view.status]);

  useEffect(() => {
    if (!asking) return;

    // Уход из окна — это `mouseout` без следующего элемента: переходы между
    // элементами внутри окна его тоже шлют, но с ним
    function leave(event: MouseEvent) {
      if (event.relatedTarget === null) setAsking(false);
    }

    document.addEventListener("mouseout", leave);
    return () => document.removeEventListener("mouseout", leave);
  }, [asking]);

  function launch(place: Place) {
    setAsking(false);
    void openSpotify(place);
  }

  const send = useCallback(async (action: Action) => {
    setFailure(null);
    try {
      await control(action);
    } catch (reason: unknown) {
      const failure = failureOf(reason);
      if (failure.kind === "nowhere") setAsking(true);
      else setFailure(failure.reason);
    }
  }, []);

  async function handleConnect(clientId: string) {
    setFailure(null);
    setConnecting(true);
    try {
      await connect(clientId);
    } catch (reason: unknown) {
      setFailure(String(reason));
    } finally {
      setConnecting(false);
    }
  }

  const panelHeight = (reference ? 180 : PLAYLISTS_HEIGHT) * geometry;

  /*
   * ВАЖНО: **окно растёт на панель ПЛЮС зазор столбца.**
   *
   * Панель встаёт в тот же флекс-столбец, что и пульт с нижним рядом, и своим
   * появлением добавляет туда ещё один `gap`. Выросшее ровно на высоту панели
   * окно этой лишней дюжины точек не покрывает — и отнимает её у пульта,
   * единственного растяжимого блока в столбце. Снаружи это и есть «виджет
   * колбасит при открытии плейлистов»: обложка при раскрытом списке на зазор
   * меньше, чем при закрытом.
   *
   * Зазор читается с самого столбца, а не пишется числом: он идёт от
   * `--space-4` через геометрию, и списанное число разошлось бы с CSS молча.
   */
  const body = useRef<HTMLDivElement | null>(null);

  // Native подгоняет от основы текущего дизайна, а не от старой прибавки.
  // Дизайн/A-B сохраняют раскрытый список; вид и размер закрывают его событием.
  const sizes = (reference ? REFERENCE_SIZES : CLAUDE_SIZES)[widgetView === 1 || widgetView === 2 ? widgetView : 0];
  const preset = sizes[Math.min(3, Math.max(0, size))] ?? sizes[0];
  const setup = view.status === "needsClientId" || view.status === "needsAuth";
  const setupExtra = reference && setup ? Math.max(0, (preset[0] < 280 ? 178 : 140) - preset[1]) * geometry : 0;
  // Вертикальный GPT встаёт высотой по содержимому: без громкости и без
  // запаса под ней (`referenceSlack`). Прибавка тогда отрицательная
  const cover = useWidgetCover();
  const shownPlayer = view.status === "active" ? view.player : view.status === "idle" ? view.last : null;
  const volume = shownPlayer !== null && hasVolume(shownPlayer);
  const slack = reference && shownPlayer !== null ? referenceSlack(widgetView, size, cover, volume) * geometry : 0;
  useEffect(() => {
    const extra = setupExtra || (connected && playlistsOpen && size < 2
      ? panelHeight + columnGap(body.current) : 0) - slack;
    growth.open(extra);
  }, [design, effectiveDesign, layout, widgetView, size, geometry, connected, playlistsOpen,
    setupExtra, panelHeight, slack, growth]);

  function togglePlaylists() {
    setPlaylistsOpen((open) => !open);
  }

  /*
   * Номер размера едет в разметку атрибутом.
   *
   * Что показывать только на больших размерах — перемешивание с повтором,
   * нижний ряд с плейлистами и устройством, — решает выбор человека, а не
   * порог по точкам: у трёх раскладок размеры разной формы, и «Большой»
   * квадрат уже «Среднего» бара по ширине, но просторнее его по площади. Порог
   * тут врал бы в двух раскладках из трёх.
   */
  return (
    <div className={reference ? "spotify spotify-reference-widget" : "spotify"}
      data-size={size} data-layout={layout === 1 ? "b" : "a"} data-format={widgetView}>
      <GlassCard accent="media" flush={reference}>
        <div className="spotify__body" ref={body}>
          <Body
            view={view}
            receivedAt={at}
            connecting={connecting}
            onConnect={handleConnect}
            send={send}
            reference={reference}
          />

          {blank !== null && <Launch text={blank} onLaunch={launch} />}
          {blank === null && asking && (
            <Launch
              // Вопрос отдельно: в самой низкой полосе он уходит целиком, и
              // остаются слова и значки
              text={t("spotify.nowhere")}
              ask={t("spotify.nowhereAsk")}
              onLaunch={launch}
              onDismiss={() => setAsking(false)}
            />
          )}

          {/* Поверх карточки, а не строкой в столбце: место пульта отказ
              занимать не должен — он живёт восемь секунд, а высоту отнимал бы
              постоянно */}
          {failure !== null && <div className="spotify__failure">{failure}</div>}

          {/* Ряд уходит порогом по высоте, а не условием тут: тесно бывает
              не раскладке, а окну — см. `@container` в spotify.css */}
          {connected && (
            <>
              <div className="spotify__actions">
                <button
                  type="button"
                  className={
                    playlistsOpen ? "spotify__link spotify__link--active" : "spotify__link"
                  }
                  onClick={togglePlaylists}
                >
                  <PlaylistIcon />
                  {t("spotify.playlists")}
                </button>
                {/* Устройство переехало сюда из шапки, которой у карточки
                    больше нет: знать, откуда идёт звук, всё равно полезно */}
                {view.status === "active" && view.player.device !== null && (
                  <span className="spotify__device">{view.player.device}</span>
                )}
              </div>

              {/*
                ВАЖНО: **высота панели — то же число, на которое выросло окно.**
                Без неё панель и пульт делят прибавку как два растяжимых блока:
                пульт сперва сжимается под новую панель, потом окно дорастает и
                он частично отыгрывает — раскладка дёргается дважды, а обложка
                при раскрытых плейлистах оказывается другого размера, чем при
                закрытых. Число одно и приходит отсюда же, а не списано в CSS.
              */}
              {playlistsOpen && (
                <PlaylistPanel
                  height={panelHeight}
                  onPlay={(uri) => void send({ kind: "playContext", uri })}
                />
              )}
            </>
          )}
        </div>
      </GlassCard>
    </div>
  );
}

/** Зазор между блоками карточки — тот самый, что добавит собой панель. */
function columnGap(element: HTMLElement | null): number {
  if (element === null) return 0;

  const gap = Number.parseFloat(window.getComputedStyle(element).rowGap);
  return Number.isFinite(gap) ? gap : 0;
}

type BodyProps = {
  view: ReturnType<typeof useSpotify>["view"];
  receivedAt: number;
  connecting: boolean;
  onConnect: (clientId: string) => void;
  send: (action: Action) => Promise<void>;
  reference: boolean;
};

function Body({ view, receivedAt, connecting, onConnect, send, reference }: BodyProps) {
  const t = useText();
  switch (view.status) {
    case "needsClientId":
      return <SetupPanel clientId="" busy={connecting} onConnect={onConnect} reference={reference} />;
    case "needsAuth":
      return <SetupPanel clientId={view.clientId} busy={connecting} onConnect={onConnect} reference={reference} />;
    case "authorizing":
      return <Note>{t("spotify.authorizing")}</Note>;
    // Пустой виджет выглядит сломанным, поэтому пока Спотифай молчит
    // показываем последний трек — блёклым. Трека не было ни разу — карточку
    // целиком занимает слой запуска с подсказкой
    case "idle":
      return view.last === null ? null : (
        <PlayerPanel player={view.last} receivedAt={receivedAt} send={send} stale />
      );
    case "active":
      return <PlayerPanel player={view.player} receivedAt={receivedAt} send={send} />;
    case "error":
      return <Note tone="danger">{view.reason}</Note>;
  }
}

type LaunchProps = {
  text: string;
  /** Вторая фраза, которая уходит первой, когда места нет. */
  ask?: string;
  onLaunch: (place: Place) => void;
  /** Нет — слой постоянный и мимо значков не закрывается. */
  onDismiss?: () => void;
};

/**
 * Запуск Спотифая поверх карточки: подсказка и два значка.
 *
 * Кнопки — значками, а не подписями: слой стоит на трёх раскладках и четырёх
 * размерах, вплоть до полосы в шестьдесят шесть точек высотой. Что делать,
 * говорит подсказка рядом, а что за кнопка — ещё и подсказка при наведении.
 *
 * В полосе подсказка слева, значки справа; в столбике и квадрате подсказка
 * над значками.
 */
function Launch({ text, ask, onLaunch, onDismiss }: LaunchProps) {
  const row = useWidgetView() === LAYOUT.row;
  const t = useText();

  // Щелчок мимо значков тоже закрывает слой: иначе убрать его, не уводя мышь
  // из окна, было бы нечем
  function dismiss(event: ReactMouseEvent) {
    if (!(event.target as Element).closest("button")) onDismiss?.();
  }

  return (
    <div className={row ? "spotify__launch spotify__launch--row" : "spotify__launch"} onClick={dismiss}>
      <p className="spotify__launch-text">
        {text}
        {ask !== undefined && <span className="spotify__launch-ask"> {ask}</span>}
      </p>
      <div className="spotify__launch-buttons">
        <button
          type="button"
          className="spotify__launch-button"
          title={t("spotify.openApp")}
          aria-label={t("spotify.openApp")}
          onClick={() => onLaunch("app")}
        >
          <SpotifyMark />
        </button>
        <button
          type="button"
          className="spotify__launch-button"
          title={t("spotify.openWeb")}
          aria-label={t("spotify.openWeb")}
          onClick={() => onLaunch("web")}
        >
          <GlobeIcon />
        </button>
      </div>
    </div>
  );
}

function Note({ children, tone }: { children: ReactNode; tone?: "danger" }) {
  return (
    <p className={tone === "danger" ? "spotify__note spotify__note--danger" : "spotify__note"}>
      {children}
    </p>
  );
}
