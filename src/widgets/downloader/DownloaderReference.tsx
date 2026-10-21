import { Bar } from "../../components/Bar";
import { Button } from "../../components/Button";
import { FolderIcon, HistoryIcon } from "../../components/Icons";
import {
  cancelDownload, isBusy, reasonText, revealDownload,
  type DownloaderView, type Mode, type Quality,
} from "../../lib/downloader";
import { duration, fileSize, transferRate } from "../../lib/format";
import { useText } from "../../lib/i18n";
import { openOutputDir } from "../../lib/output";

type Props = {
  view: DownloaderView;
  layout: "full" | "rows";
  url: string;
  refusal: string | null;
  dismissed: boolean;
  shown: boolean;
  busyText: string;
  detailText: string;
  onUrl: (value: string) => void;
  onStart: () => void;
  onChoose: (mode: Mode, quality: Quality) => void;
  onDismiss: () => void;
  onHistory: () => void;
};

const QUALITY: Quality[] = ["p720", "p1080", "best"];

/** Утверждённый A: только представление общей модели задания и ввода. */
export function DownloaderReference(props: Props) {
  const { view, url, shown, onUrl, onStart, onChoose, onHistory } = props;
  const t = useText();
  const busy = isBusy(view.status);
  const qualityName = (quality: Quality) =>
    quality === "best" ? t("downloader.best") : quality === "p720" ? "720p" : "1080p";

  return (
    <>
      <input type="text" className="input dl-a__url" value={url}
        aria-label={t("downloader.placeholder")} placeholder={t("downloader.placeholder")}
        spellCheck={false} autoComplete="off" data-no-drag disabled={busy}
        onChange={(event) => onUrl(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !busy && url.trim() !== "") onStart();
        }} />

      <span className="dl-a__go">
        {busy ? (
          <Button variant="outline" onClick={() => void cancelDownload()}>{t("common.cancel")}</Button>
        ) : (
          <Button onClick={onStart} disabled={url.trim() === ""}>{t("tools.gate.download")}</Button>
        )}
      </span>

      <div className="dl-a__options">
        <div className="dl-a__mode">
          <button type="button" aria-pressed={view.mode === "video"}
            title={t("downloader.video") + " · MP4"} onClick={() => onChoose("video", view.quality)}>
            {t("downloader.video")}
          </button>
          <button type="button" aria-pressed={view.mode === "audio"}
            title={t("downloader.audio") + " · MP3 · 192 kbps"} onClick={() => onChoose("audio", view.quality)}>
            {t("downloader.audio")}
          </button>
        </div>
        <div className="dl-a__quality">
          {view.mode === "video" ? (
            <>
              <div className="dl-a__quality-buttons" role="group" aria-label={t("downloader.qualityCeiling")}>
                {QUALITY.map((quality) => (
                  <button key={quality} type="button" aria-pressed={view.quality === quality}
                    title={t("downloader.qualityCeiling") + ": " + qualityName(quality)}
                    onClick={() => onChoose("video", quality)}>
                    {qualityName(quality)}
                  </button>
                ))}
              </div>
              <select className="dl-a__quality-select" aria-label={t("downloader.qualityCeiling")}
                title={t("downloader.qualityCeiling")} value={view.quality}
                onChange={(event) => onChoose("video", event.target.value as Quality)}>
                {QUALITY.map((quality) => <option key={quality} value={quality}>{qualityName(quality)}</option>)}
              </select>
            </>
          ) : (
            <span className="dl-a__audio-format">MP3 · 192 kbps</span>
          )}
        </div>
        <div className="dl-a__utilities">
          <span className="dl-a__folder">
            <Button variant="outline" icon={<FolderIcon />} label={t("downloader.openFolder")}
              onClick={() => void openOutputDir("downloader").catch(() => {})} />
          </span>
          {view.history.length > 0 && (
            <span className="dl-a__history-toggle" data-open={shown}>
              <Button variant="outline" icon={<HistoryIcon />}
                label={shown ? t("downloader.hideHistory") : t("downloader.showHistory")} onClick={onHistory}>
                {view.history.length}
              </Button>
            </span>
          )}
        </div>
      </div>

      <ReferenceStatus {...props} />
    </>
  );
}

/** Числа идут целыми фрагментами; длинный ряд переносится и растит основу. */
function DownloadMetrics({ view, busyText }: Pick<Props, "view" | "busyText">) {
  const t = useText();
  if (view.status !== "downloading") {
    return <div className="dl-a__status-meta"><span>{busyText}</span></div>;
  }

  const part = view.part !== null && view.part.count > 1
    ? t(view.part.index === 1 ? "downloader.video" : "downloader.sound") : null;
  const percent = view.percent === null ? null : Math.round(view.percent * 100) + "%";
  const head = [part, percent].filter(Boolean).join(" ");
  const speed = view.speed === null ? null : transferRate(view.speed);
  const size = view.downloaded === null ? null : view.total === null ? fileSize(view.downloaded)
    : fileSize(view.downloaded) + " / " + fileSize(view.total);
  const eta = view.eta === null ? null : duration(view.eta);

  return (
    <div className="dl-a__status-meta tabular">
      <span className="dl-a__metric-group">
        {head !== "" && <span>{head}</span>}
        {speed !== null && <span>{speed}</span>}
        {head === "" && speed === null && <span>{t("downloader.busy.downloading")}</span>}
      </span>
      {(size !== null || eta !== null) && (
        <span className="dl-a__metric-group dl-a__metric-group--extent">
          {size !== null && <span>{size}</span>}
          {eta !== null && <span>{eta}</span>}
        </span>
      )}
    </div>
  );
}

function ReferenceStatus({
  view, layout, refusal, dismissed, busyText, detailText, onDismiss,
}: Props) {
  const t = useText();
  const busy = isBusy(view.status);
  const failure = refusal ?? (view.status === "failed"
    ? reasonText(t, view.reason ?? "failed", view.detail) : null);
  const failed = failure !== null && !dismissed;
  const phase = failed ? "failed" : busy ? "busy"
    : view.status === "done" ? "done" : view.status === "cancelled" ? "cancelled" : "idle";
  const title = failed ? failure : phase === "done" ? view.output ?? view.title ?? t("downloader.saved")
    : busy ? view.title ?? t("downloader.getting")
    : phase === "cancelled" ? t("tools.failure.cancelled") : t("downloader.idle");
  const hint = failed ? [failure, view.detail, t("downloader.dismissFailure")].filter(Boolean).join("\n")
    : busy ? [view.title, detailText, busyText].filter(Boolean).join("\n")
    : phase === "done" ? view.output ?? undefined : view.outDir ?? undefined;
  const fill = view.status === "downloading" ? view.percent : null;

  if (layout === "rows") {
    if (!busy && !failed) return null;
    const part = view.part !== null && view.part.count > 1
      ? t(view.part.index === 1 ? "downloader.video" : "downloader.sound") : null;
    const percent = view.percent === null ? null : Math.round(view.percent * 100) + "%";
    const head = [part, percent].filter(Boolean).join(" ");
    const speed = view.status === "downloading" && view.speed !== null ? transferRate(view.speed) : null;
    return (
      <div className="dl-a__overlay" data-phase={phase} title={hint}>
        {failed ? (
          <button type="button" className="dl-a__dismiss" aria-label={t("downloader.dismissFailure")}
            title={hint} onClick={onDismiss}>
            <span>{failure}</span><span aria-hidden="true">✕</span>
          </button>
        ) : (
          <div className="dl-a__overlay-line">
            {view.status === "downloading" ? (
              <>
                {(head !== "" || speed === null) && (
                  <span className="dl-a__overlay-value tabular">{head || t("downloader.busy.downloading")}</span>
                )}
                {speed !== null && <span className="dl-a__overlay-value tabular">{speed}</span>}
              </>
            ) : <span>{busyText}</span>}
          </div>
        )}
        {busy && <div className="busy-bar dl-a__overlay-bar"><Bar fill={fill} thickness="thin" /></div>}
      </div>
    );
  }

  return (
    <div className="dl-a__status" data-phase={phase} title={hint}>
      {failed ? (
        <button type="button" className="dl-a__full-dismiss" title={hint} onClick={onDismiss}>
          <span className="dl-a__status-title">{failure}</span>
          <span className="dl-a__status-hint">{t("downloader.dismissFailure")}</span>
        </button>
      ) : (
        <>
          <span className="dl-a__status-title">{title}</span>
          {phase === "done" ? (
            <button type="button" className="dl-a__show" onClick={() => void revealDownload()}>
              {t("downloader.showInFolder")}
            </button>
          ) : busy ? (
            <DownloadMetrics view={view} busyText={busyText} />
          ) : (
            <span className="dl-a__status-hint">{view.outDir ?? t("tools.noDesktop")}</span>
          )}
        </>
      )}
      {busy && <div className="busy-bar dl-a__status-bar"><Bar fill={fill} thickness="thin" /></div>}
    </div>
  );
}
