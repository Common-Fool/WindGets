import { Bar } from "../../components/Bar";
import { Button } from "../../components/Button";
import { CloseIcon, ConvertIcon, FolderIcon, PlusIcon } from "../../components/Icons";
import {
  cancelConverting,
  clearFiles,
  pickFiles,
  removeFile,
  revealConverted,
  startConverting,
  type ConverterView,
  type Item,
} from "../../lib/converter";
import { useText } from "../../lib/i18n";
import { openOutputDir } from "../../lib/output";

/** GPT A: существующие действия и показания. Состояние пачки остаётся у Rust. */
export function ConverterReferenceHeader({ view }: { view: ConverterView }) {
  const t = useText();
  const ready = view.items.filter((item) => item.status === "ready").length;
  const done = view.items.filter((item) => item.status === "done").length;
  const summary = `${t.count("converter.files", view.items.length)} · ${t(
    view.running ? "converter.doneCount" : "converter.readyCount",
    { count: view.running ? done : ready },
  )}`;

  return (
    <div className="conv-ref__head">
      <span className="conv-ref__summary tabular" title={summary}>{summary}</span>
      <Button variant="outline" icon={<PlusIcon />} label={t("converter.add")} disabled={view.running} onClick={() => void pickFiles()} />
      <Button variant="outline" icon={<CloseIcon />} label={t("converter.clearList")} disabled={view.running} onClick={() => void clearFiles()} />
      <Button variant="outline" icon={<FolderIcon />} label={t("converter.openFolder")} disabled={view.running} onClick={() => void openOutputDir("converter")} />
    </div>
  );
}

/** Путь — существующий outDir. Проценты показываем только из настоящего снимка. */
export function ConverterReferenceFooter({ view, canStart }: { view: ConverterView; canStart: boolean }) {
  const t = useText();
  return (
    <div className="conv-ref__foot">
      {view.running ? (
        <>
          {view.overall !== null && <span className="conv-ref__percent tabular">{Math.round(view.overall * 100)}%</span>}
          <div className="busy-bar conv__overall"><Bar fill={view.overall} thickness="thin" /></div>
          <Button variant="outline" onClick={() => void cancelConverting()}>{t("common.cancel")}</Button>
        </>
      ) : (
        <>
          <span className="conv-ref__path" title={view.outDir ?? t("tools.noDesktop")}>{view.outDir ?? t("tools.noDesktop")}</span>
          <Button icon={<ConvertIcon />} onClick={() => void startConverting().catch(() => {})} disabled={!canStart}>
            {t("converter.convert")}
          </Button>
        </>
      )}
    </div>
  );
}

/** Строка GPT не скрывает причину отказа; состояние и команды переданы общим Row. */
export function ConverterReferenceRow({ item, running, state }: { item: Item; running: boolean; state: string }) {
  const t = useText();
  const actionLabel = item.status === "done"
    ? t("converter.reveal", { name: item.output ?? t("converter.theFile") })
    : t("common.remove");
  return (
    <li className="conv__item" data-status={item.status}>
      <span className="conv__name" title={item.name}>{item.name}</span>
      <span className="conv__state tabular" title={item.reason ?? item.output ?? undefined}>{state}</span>
      <button
        type="button"
        className={item.status === "done" ? "conv__action" : "conv__action conv__action--remove"}
        title={actionLabel}
        aria-label={actionLabel}
        disabled={item.status !== "done" && running}
        onClick={() => void (item.status === "done" ? revealConverted(item.id) : removeFile(item.id))}
      >
        {item.status === "done" ? <FolderIcon /> : <CloseIcon />}
      </button>
      {item.reason !== null && <p className="conv-ref__reason" data-no-drag>{item.reason}</p>}
      {item.status === "running" && <div className="busy-bar conv__bar"><Bar fill={item.progress} thickness="thin" /></div>}
    </li>
  );
}

export function ConverterReferenceEmpty() {
  const t = useText();
  return (
    <div className="conv__empty">
      <p className="conv__lead">{t("converter.dropHere")}</p>
      <Button icon={<PlusIcon />} onClick={() => void pickFiles()}>{t("converter.choose")}</Button>
      <div className="conv-ref__formats">
        <span>{t("converter.state.video")}</span><span>→ MP4 · GIF · MP3</span>
        <span>{t("converter.state.audio")}</span><span>→ MP3</span>
        <span>{t("converter.state.image")}</span><span>→ PNG · JPG</span>
      </div>
    </div>
  );
}
