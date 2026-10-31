import { useEffect, useRef, useState } from "react";
import { copyToClipboard } from "../../lib/copy";
import { duration, fileSize, transferRate } from "../../lib/format";
import { useContentHeight, useWindowGrowth } from "../../lib/grow";
import { useLook, useScale } from "../../lib/look";
import { openOutputDir } from "../../lib/output";
import { useWindowResized } from "../../lib/resized";
import { useHeight } from "../../lib/size";
import { missingFor, useTools } from "../../lib/tools";
import { useWidgetSize, useWidgetView } from "../../lib/view";
import {
  cancelDownload,
  forgetDownload,
  isBusy,
  reasonText,
  revealDownload,
  setDownloadOptions,
  startDownload,
  useDownloader,
  type DownloaderView,
  type Mode,
  type Quality,
  type Saved,
} from "../../lib/downloader";
import { Bar } from "../../components/Bar";
import { Button } from "../../components/Button";
import { GlassCard } from "../../components/GlassCard";
import { CopyIcon, DoneIcon, FolderIcon, HistoryIcon } from "../../components/Icons";
import { Toggle } from "../../components/Toggle";
import { ToolsGate } from "../../components/ToolsGate";
import "./downloader.css";
import { DownloaderReference } from "./DownloaderReference";
import "./downloader-reference.css";
import { useText, type Text } from "../../lib/i18n";

const QUALITIES: { id: Quality; label: string }[] = [
  { id: "p720", label: "720p" },
  { id: "p1080", label: "1080p" },
  // «Лучшее» переводится, числа — нет: подпись берётся в самом ряду
  { id: "best", label: "downloader.best" },
];

/**
 * Зазор между рядом кнопок и списком при 100 %, если сетка его не отдала, —
 * до первого замера. Дальше он берётся у самой сетки: у двух раскладок он
 * разный.
 */
const HISTORY_GAP = 8;

/** Сколько держим галочку «скопировано» на месте значка. */
const COPIED_MS = 1200;

/**
 * Загрузчик: видео или звук по ссылке.
 *
 * Любая ссылка, которую понимает yt-dlp, — не только YouTube. Режим и потолок
 * качества помнит Rust до выхода из приложения, в файл они не пишутся.
 *
 * Видов два, и это виды, а не размеры одного вида: «В две строки» — ссылка с
 * кнопкой скачивания сверху, выбор и кнопки папки и истории снизу, без строки
 * состояния. Раскрытая история растит окно, и порог по высоте перебрасывал бы
 * раскрытую маленькую карточку в полную раскладку — вид от высоты не зависит.
 * У Classic раскладку решает прежняя сетка (`downloader.css`), у NEW —
 * утверждённый A. Ввод и история общие, смена оформления их не сбрасывает.
 * Вид меняют пунктом меню и в настройках виджета.
 */
export function DownloaderWidget() {
  const tools = useTools();
  const view = useDownloader();
  const widgetView = useWidgetView();
  const size = useWidgetSize();
  const layout = widgetView === 1 ? "rows" : "full";
  const { effectiveDesign, scale } = useLook();
  const reference = effectiveDesign === "reference";
  const t = useText();
  const content = useRef<HTMLDivElement>(null);
  const ready = missingFor(tools, "downloader").length === 0 && !tools.probing;

  // У NEW один владелец высоты: основа, согласие и раскрытый список меряются
  // вместе. Очередь прошлого дизайна отменяется существующим хуком.
  useContentHeight(content, reference,
    [effectiveDesign, widgetView, size, ready, scale, t.language].join("|"), true);

  return (
    <div className={reference ? "widget downloader-reference-widget" : "widget downloader-app-widget"}>
      <GlassCard accent="neutral" flush={reference}>
        <div className={reference ? "dl dl-a" : "dl"} data-layout={layout}
          data-ready={ready} ref={content}>
          <Panel view={view} ready={ready} reference={reference} layout={layout} />
        </div>
      </GlassCard>
    </div>
  );
}

function Panel({ view, ready, reference, layout }: {
  view: DownloaderView; ready: boolean; reference: boolean; layout: "full" | "rows";
}) {
  const t = useText();
  const [url, setUrl] = useState("");
  const [refusal, setRefusal] = useState<string | null>(null);
  const busy = isBusy(view.status);

  /*
   * Скачалось — поле пустеет, ссылка уезжает в историю. Смотрим на переход в
   * «готово», а не на само «готово»: окно, открытое после загрузки, стирать
   * нечего, а неудачная загрузка ссылку оставляет — её повторят.
   */
  const was = useRef(view.status);
  useEffect(() => {
    if (view.status === "done" && was.current !== "done") setUrl("");
    was.current = view.status;
  }, [view.status]);

  /*
   * Отказ, убранный нажатием. В виде «В две строки» он лежит поверх поля
   * ссылки и закрывал бы его до следующей загрузки — нажатие по нему
   * возвращает поле. Новый отказ показывается снова.
   */
  const [dismissed, setDismissed] = useState(false);
  useEffect(() => setDismissed(false), [view.status, refusal]);

  const [historyOpen, setHistoryOpen] = useState(false);
  const history = view.history;
  // Очищенная история сворачивает и панель: пустой раскрытой ей быть незачем
  const shown = historyOpen && history.length > 0;
  // Очищенный список сворачивается сам: иначе флаг оставался поднятым, и
  // следующая загрузка раскрывала бы список без спроса, растя окно
  if (historyOpen && history.length === 0) setHistoryOpen(false);

  function choose(mode: Mode, quality: Quality) {
    void setDownloadOptions(mode, quality);
  }

  async function start() {
    setRefusal(null);
    try {
      await startDownload(url, view.mode, view.quality);
    } catch (reason) {
      setRefusal(String(reason) === "badUrl" ? t("downloader.badUrl") : null);
    }
  }

  // Panel не перемонтируется при смене оформления или готовности программ:
  // набранный URL и отказ остаются у одного владельца.
  if (!ready) return <ToolsGate module="downloader" />;

  return (
    <>
      {reference ? (
        <DownloaderReference view={view} layout={layout} url={url}
          refusal={refusal} dismissed={dismissed} shown={shown}
          busyText={busyLine(t, view)} detailText={downloadLine(t, view)}
          onUrl={(value) => { setUrl(value); setRefusal(null); }}
          onStart={() => void start()} onChoose={choose}
          onDismiss={() => setDismissed(true)} onHistory={() => setHistoryOpen(!shown)} />
      ) : (
      <>
      <input
        type="text"
        className="input dl__url"
        value={url}
        placeholder={t("downloader.placeholder")}
        spellCheck={false}
        autoComplete="off"
        data-no-drag
        disabled={busy}
        onChange={(event) => {
          setUrl(event.target.value);
          setRefusal(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !busy && url.trim() !== "") void start();
        }}
      />

      <div className="dl__options">
        <div className="dl__chips">
          <Toggle
            label={t("downloader.video")}
            short="MP4"
            active={view.mode === "video"}
            onChange={() => choose("video", view.quality)}
          />
          <Toggle
            label={t("downloader.audio")}
            short="MP3"
            active={view.mode === "audio"}
            onChange={() => choose("audio", view.quality)}
          />
        </div>
        {/* Потолок у звука не значит ничего — он и не показывается */}
        {view.mode === "video" && (
          <div className="dl__chips">
            {QUALITIES.map((quality) => (
              <Toggle
                key={quality.id}
                label={quality.id === "best" ? t("downloader.best") : quality.label}
                short={quality.id === "best" ? "★" : quality.label}
                active={view.quality === quality.id}
                onChange={() => choose("video", quality.id)}
              />
            ))}
          </div>
        )}
      </div>

      <Status view={view} refusal={refusal} dismissed={dismissed} onDismiss={() => setDismissed(true)} />

      {/* Кнопки — прямо в сетке, каждая в своей клетке: в двух раскладках
          они стоят в разных местах */}
      <span className="dl__folder">
        <Button
          variant="outline"
          icon={<FolderIcon />}
          label={t("downloader.openFolder")}
          onClick={() => void openOutputDir("downloader").catch(() => {})}
        />
      </span>
      {history.length > 0 && (
        <span className="dl__toggle">
          <Button
            variant="outline"
            icon={<HistoryIcon />}
            label={shown ? t("downloader.hideHistory") : t("downloader.showHistory")}
            onClick={() => setHistoryOpen(!shown)}
          >
            {history.length}
          </Button>
        </span>
      )}
      <span className="dl__go">
        {busy ? (
          <Button variant="outline" onClick={() => void cancelDownload()}>
            {t("common.cancel")}
          </Button>
        ) : (
          <Button onClick={() => void start()} disabled={url.trim() === ""}>
            {t("tools.gate.download")}
          </Button>
        )}
      </span>

      </>
      )}
      {/* История вне веток дизайна: строки и таймер копирования сохраняются. */}
      <History history={shown ? history : []} reference={reference}
        onClose={() => setHistoryOpen(false)} />
    </>
  );
}

/**
 * Скачанное за этот запуск — раскрывается под нижним рядом, и окно растёт под
 * него (`useWindowGrowth`, как плейлисты у пульта и прогноз у погоды). Свёрнутая панель —
 * это пустой список: хук роста тогда возвращает окну прежнюю высоту.
 *
 * Строка — название; нажатие показывает файл в папке. Рядом — копирование
 * ссылки и крестик, внизу — очистка всего списка. Сами файлы список не
 * трогает: забывается только он.
 */
function History({ history, onClose, reference }: {
  history: Saved[]; onClose: () => void; reference: boolean;
}) {
  const t = useText();
  const growth = useWindowGrowth();
  const geometry = useScale();
  const panel = useRef<HTMLDivElement>(null);
  const height = useHeight(panel);
  const open = history.length > 0;

  // Готовый размер или вид сменили — раскрытой панели в нём места нет
  useWindowResized(onClose);

  useEffect(() => {
    // NEW меряет всё содержимое в DownloaderWidget. Старый долг на смене
    // дизайна забывает общий useWindowGrowth, без вычитания из новой основы.
    if (reference) return;
    if (!open) {
      growth.close();
      return;
    }
    if (height <= 0) return;
    // Меряем саму панель, а не считаем строки: они едут за кеглем. Зазор над
    // ней — у сетки, и у двух раскладок он свой
    const grid = panel.current?.parentElement;
    const gap = grid ? parseFloat(getComputedStyle(grid).rowGap) : NaN;
    growth.open(height + (Number.isFinite(gap) ? gap : HISTORY_GAP * geometry));
  }, [open, height, geometry, growth, reference]);

  if (!open) return null;

  return (
    <div className="dl__history" ref={panel}>
      <ul className="dl__saved">
        {history.map((saved) => (
          <SavedRow key={saved.id} saved={saved} />
        ))}
      </ul>
      <div className="dl__history-foot">
        {reference && <span className="dl-a__files-remain">{t("downloader.filesRemain")}</span>}
        <button type="button" className="dl__clear" onClick={() => void forgetDownload()}>
          {t("converter.clearList")}
        </button>
      </div>
    </div>
  );
}

function SavedRow({ saved }: { saved: Saved }) {
  const t = useText();
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  async function copy() {
    if (!(await copyToClipboard(saved.url))) return;
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  }

  return (
    <li className="dl__saved-item">
      <button
        type="button"
        className="dl__saved-title"
        title={`${saved.title}\n${saved.url}\n${t("converter.reveal", { name: saved.file })}`}
        onClick={() => void revealDownload(saved.id)}
      >
        {saved.title}
      </button>
      <span className="dl__saved-kind">{saved.mode === "audio" ? "MP3" : "MP4"}</span>
      <button
        type="button"
        className="dl__saved-action"
        title={copied ? t("downloader.linkCopied") : t("downloader.copyLink")}
        onClick={() => void copy()}
      >
        {copied ? <DoneIcon /> : <CopyIcon />}
      </button>
      <button
        type="button"
        className="dl__saved-action dl__saved-drop"
        title={t("downloader.forget")}
        onClick={() => void forgetDownload(saved.id)}
      >
        ✕
      </button>
    </li>
  );
}

/** Чем занята строка состояния — от этого её цвет и место в виде «В две строки». */
type Phase = "idle" | "busy" | "done" | "failed" | "cancelled";

/**
 * Строка состояния — всегда одна строка: слева что (название, причина,
 * подсказка), справа ход работы или «Show in folder». Полоса хода лежит под
 * ней в зазоре до кнопок. В две строки она тут и росла прежде — название,
 * полоса, числа, — и под любым другим состоянием оставалась пустота на целую
 * строку: её и просили убрать.
 *
 * В виде «В две строки» своей строки нет: работа и отказ ложатся поверх поля
 * ссылки, а готовое и тишина не показываются вовсе — готовое есть в истории.
 */
function Status({
  view,
  refusal,
  dismissed,
  onDismiss,
}: {
  view: DownloaderView;
  refusal: string | null;
  dismissed: boolean;
  onDismiss: () => void;
}) {
  const t = useText();
  let phase: Phase;
  let text: string;
  let side: string | null = null;
  let hint: string | undefined;
  let fill: number | null = null;

  if (refusal !== null) {
    phase = "failed";
    text = refusal;
  } else {
    switch (view.status) {
      case "idle":
        phase = "idle";
        text = t("downloader.idle");
        break;
      case "preparing":
      case "updating":
      case "processing":
      case "saving":
      case "downloading":
        phase = "busy";
        text = view.title ?? t("downloader.getting");
        hint = view.title ?? undefined;
        side = busyLine(t, view);
        if (view.status === "downloading") {
          fill = view.percent;
          hint = [view.title, downloadLine(t, view)].filter(Boolean).join("\n");
        }
        break;
      case "done":
        phase = "done";
        text = view.output ?? view.title ?? t("downloader.saved");
        hint = view.output ?? undefined;
        break;
      case "failed":
        phase = "failed";
        text = reasonText(t, view.reason ?? "failed", view.detail);
        // Только наш текст: у разобранного отказа сырой вывод yt-dlp — шум на
        // три строки (22 октября, слово пользователя), у неразобранного он и
        // есть текст. Подробности — в журнале
        hint = text;
        break;
      case "cancelled":
        phase = "cancelled";
        text = t("tools.failure.cancelled");
        break;
    }
  }

  // Убранный отказ — снова тишина: поле ссылки открыто, а строка пустеет
  if (phase === "failed" && dismissed) {
    phase = "idle";
    text = t("downloader.idle");
    hint = undefined;
  }

  return (
    <div
      className="dl__status"
      data-phase={phase}
      title={hint}
      onClick={phase === "failed" ? onDismiss : undefined}
    >
      <span className="dl__status-text">{text}</span>
      {side !== null && <span className="dl__status-side tabular">{side}</span>}
      {phase === "done" && (
        <button type="button" className="dl__show" onClick={() => void revealDownload()}>
          {t("downloader.showInFolder")}
        </button>
      )}
      {phase === "busy" && (
        <div className="busy-bar dl__status-bar">
          <Bar fill={fill} thickness="thin" />
        </div>
      )}
    </div>
  );
}

/** Что сейчас делается — коротко, в правый край строки: «Video 45% · 3.2 MB/s». */
function busyLine(t: Text, view: DownloaderView): string {
  switch (view.status) {
    case "preparing":
      return t("downloader.busy.preparing");
    case "updating":
      return t("downloader.busy.updating");
    case "processing":
      return view.step === "converting" ? t("downloader.busy.converting") : t("downloader.busy.joining");
    case "saving":
      return t("common.saving");
    default: {
      const parts: string[] = [];
      const part =
        view.part !== null && view.part.count > 1
          ? t(view.part.index === 1 ? "downloader.video" : "downloader.sound")
          : null;
      const percent = view.percent !== null ? `${Math.round(view.percent * 100)}%` : null;
      const head = [part, percent].filter(Boolean).join(" ");
      if (head !== "") parts.push(head);
      if (view.speed !== null) parts.push(transferRate(view.speed));
      return parts.length > 0 ? parts.join(" · ") : t("downloader.busy.downloading");
    }
  }
}

/** Всё о загрузке целиком — в подсказку: «Video · 12 / 45 MB · 3.2 MB/s · 0:12». */
function downloadLine(t: Text, view: DownloaderView): string {
  const parts: string[] = [];
  if (view.part !== null && view.part.count > 1) {
    parts.push(t(view.part.index === 1 ? "downloader.video" : "downloader.sound"));
  }
  if (view.downloaded !== null) {
    parts.push(
      view.total !== null
        ? `${fileSize(view.downloaded)} / ${fileSize(view.total)}`
        : fileSize(view.downloaded),
    );
  }
  if (view.speed !== null) parts.push(transferRate(view.speed));
  if (view.eta !== null) parts.push(duration(view.eta));
  return parts.join(" · ");
}
