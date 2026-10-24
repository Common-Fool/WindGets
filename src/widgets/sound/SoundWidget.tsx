import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { useWindowResized } from "../../lib/resized";
import { useWindowDrag } from "../../lib/drag";
import { useText } from "../../lib/i18n";
import { usePresetWindowGrowth, useSoundSidePanel } from "../../lib/grow";
import { useHeight, useWidth } from "../../lib/size";
import { useLook } from "../../lib/look";
import { setSoundMixerCollapsed, useOwnRow, useSettings } from "../../lib/settings";
import { GlassCard } from "../../components/GlassCard";
import { Slider } from "../../components/Slider";
import {
  askIcon, control, iconOf, silent, useSound,
  type Device, type Panel, type Track,
} from "../../lib/sound";
import {
  ChevronIcon, CycleIcon, DeviceIcon, MicrophoneIcon, MicrophoneMutedIcon,
  MixerIcon, SpeakerIcon, SpeakerMutedIcon,
} from "./icons";
import "./sound.css";

/** Опрос не должен перебивать значение под мышью до подтверждения команды. */
const HOLD_MS = 400;

/**
 * Сколько ждём подтверждения команды, прежде чем вернуть то, что говорит
 * система: команда могла не пройти (отказ драйвера), а громкость — встать
 * округлённой (просили 43, стоит 42). Rust тогда не шлёт ничего нового.
 */
const SETTLE_MS = 600;

function useAhead<T>(polled: T): [T, (next: T) => void] {
  const [shown, setShown] = useState(polled);
  const sentAt = useRef(0);
  const latest = useRef(polled);
  latest.current = polled;
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEffect(() => {
    const left = HOLD_MS - (Date.now() - sentAt.current);
    if (left <= 0) {
      setShown(polled);
      return;
    }
    // Пришло в защитный срок — не теряем, а применяем по его концу: иначе
    // снимок другого устройства, пришедший сразу после правки, не лёг бы
    // никогда — одинаковых снимков Rust повторно не шлёт
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setShown(latest.current), left);
  }, [polled]);

  return [shown, (next) => {
    sentAt.current = Date.now();
    setShown(next);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setShown(latest.current), HOLD_MS + SETTLE_MS);
  }];
}

type ButtonProps = {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  className?: string;
  quiet?: boolean;
  disabled?: boolean;
  expanded?: boolean;
  onClick: () => void;
};

/** Одинаковые кнопки во всех трёх раскладках; цвет занят состоянием. */
function SoundButton({ icon, title, children, className = "", quiet = false,
  disabled = false, expanded, onClick }: ButtonProps) {
  // После нажатия мышь остаётся на кнопке, и значок-подсказка (`Flip`) тут же
  // показал бы прежнее состояние — будто нажатие не сработало. Подсказка
  // молчит, пока мышь не уйдёт с кнопки.
  const [pressed, setPressed] = useState(false);
  return (
    <button type="button" className={`sound__button ${className}`} title={title}
      aria-label={title} aria-expanded={expanded} data-quiet={quiet} data-pressed={pressed || undefined}
      disabled={disabled} onClick={() => { setPressed(true); onClick(); }}
      onMouseLeave={() => setPressed(false)}>
      <span className="sound__glyph">{icon}</span>
      {children}
    </button>
  );
}

/**
 * Значок, который под курсором показывает, чем станет по нажатию: звучащий —
 * перечёркнутым, заглушённый — обычным. Обмен ведёт CSS и только в столбике.
 */
function Flip({ now, next }: { now: ReactNode; next: ReactNode }) {
  return <><span className="sound__flip-now">{now}</span><span className="sound__flip-next">{next}</span></>;
}

function DeviceRow({ device, input, onPick }: { device: Device; input: boolean; onPick: () => void }) {
  return (
    <li>
      <button type="button" className="sound__device" data-current={device.current}
        aria-pressed={device.current} title={device.name} onClick={onPick}>
        <span className="sound__glyph">{input ? <MicrophoneIcon /> : <DeviceIcon kind={device.kind} />}</span>
        <span className="sound__device-name">{device.short}</span>
        {device.current && <span className="sound__selected" aria-hidden="true">✓</span>}
      </button>
    </li>
  );
}

/** Значок из exe: отдельный запрос и кэш, вне событий с громкостью. */
function AppIcon({ name }: { name: string }) {
  const icon = iconOf(name);
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (icon === undefined) { askIcon(name); return; }
    if (icon === null) return;
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    // Битый ответ не должен уронить всю карточку из эффекта; очищаем и
    // прежний рисунок, если этот canvas уже показывал другой значок.
    context.clearRect(0, 0, context.canvas.width, context.canvas.height);
    try {
      const bytes = Uint8ClampedArray.from(atob(icon.pixels), (letter) => letter.charCodeAt(0));
      context.putImageData(new ImageData(bytes, icon.width, icon.height), 0, 0);
    } catch {
      // Пустая плашка остаётся на месте значка.
    }
  }, [icon, name]);
  if (icon === undefined || icon === null) return <span className="sound__app-icon sound__app-fallback"><MixerIcon /></span>;
  return <canvas className="sound__app-icon" ref={canvas} width={icon.width} height={icon.height} />;
}

function TrackRow({ track }: { track: Track }) {
  const t = useText();
  const [shown, show] = useAhead(track);
  const quiet = shown.muted || shown.volume === 0;
  const hint = t(quiet ? "sound.unmuteApp" : "sound.muteApp", { name: track.name });
  function change(percent: number) {
    show({ ...shown, volume: percent, muted: percent > 0 ? false : shown.muted });
    control({ kind: "sessionVolume", id: track.id, percent });
  }
  function toggle() {
    // Тишина нулём, а не флагом: снятие флага её не вернёт — как у общей
    // громкости, возвращаем полную
    if (quiet && shown.volume === 0) {
      show({ ...shown, volume: 100, muted: false });
      control({ kind: "sessionVolume", id: track.id, percent: 100 });
      if (shown.muted) control({ kind: "sessionMute", id: track.id, muted: false });
      return;
    }
    show({ ...shown, muted: !quiet });
    control({ kind: "sessionMute", id: track.id, muted: !quiet });
  }
  return (
    <li className="sound__track" data-quiet={quiet}>
      <button type="button" className="sound__track-icon" title={hint} aria-label={hint} onClick={toggle}>
        <AppIcon name={track.id} />
      </button>
      <div className="sound__track-body">
        <span className="sound__track-name" title={track.name}>{track.name}</span>
        <div className="sound__track-volume">
          <Slider value={quiet ? 0 : shown.volume} min={0} max={100} onChange={change}
            ariaLabel={t("sound.appVolume", { name: track.name })} />
          <span className="sound__percent tabular">{quiet ? 0 : shown.volume} %</span>
        </div>
      </div>
      <SoundButton className="sound__track-mute" icon={quiet ? <SpeakerMutedIcon /> : <SpeakerIcon />}
        quiet={quiet} title={hint} onClick={toggle} />
    </li>
  );
}

export function SoundWidget() {
  const design = useLook().effectiveDesign;
  useWindowDrag();
  const t = useText();
  const state = useSound();
  const own = useOwnRow();
  const settings = useSettings();
  const { scale } = useLook();
  const view = own?.view ?? 0;
  const size = own?.size ?? 0;
  const layout = `${view}:${size}`;
  const [output, showOutput] = useAhead(state.output);
  const [microphone, showMicrophone] = useAhead(state.microphone);
  const [overlay, setOverlay] = useState<{ layout: string; panel: Panel }>({ layout, panel: "none" });
  const [pendingCollapsed, setPendingCollapsed] = useState<boolean | null>(null);
  const collapsed = pendingCollapsed ?? settings.soundMixerCollapsed;
  const [resized, setResized] = useState(0);
  const growth = usePresetWindowGrowth(view, size);

  useEffect(() => { setOverlay({ layout, panel: "none" }); }, [layout]);

  useEffect(() => {
    if (pendingCollapsed === settings.soundMixerCollapsed) setPendingCollapsed(null);
  }, [pendingCollapsed, settings.soundMixerCollapsed]);

  // Смена вида убирает временную панель, мини-микшер восстанавливает предпочтение.
  useWindowResized(() => {
    setOverlay({ layout, panel: "none" });
    setResized((value) => value + 1);
  });

  const temporary = overlay.layout === layout ? overlay.panel : "none";
  const panel: Panel = own === undefined ? "none"
    : temporary === "none" && view === 0 && !collapsed ? "mixer" : temporary;
  useEffect(() => {
    // До своих настроек неизвестны ни вид, ни сохранённое раскрытие.
    if (own === undefined) return;
    control({ kind: "panel", panel });
  }, [own !== undefined, panel]);

  const panelBox = useRef<HTMLDivElement>(null);
  const opened = useHeight(panelBox);
  const panelWidth = useWidth(panelBox);
  const side = useSoundSidePanel(view, size, scale,
    panel === "none" ? 0 : panelWidth, panel === "none" ? 0 : opened + 2);
  // У правого края основа зеркальная: шкала у края, стрелки смотрят туда,
  // куда раскроется панель
  const mirrored = view === 2 && side === "left";
  useEffect(() => {
    // Пока строка настроек не приехала, готовый размер ещё неизвестен.
    if (own === undefined || view === 2) return;
    if (panel === "none") growth.close();
    else if (opened > 0) growth.open(opened);
  }, [own !== undefined, view, panel, opened, growth, resized]);

  const quiet = silent(output);
  const current = state.current;
  const rows = panel === "outputs" ? state.devices : panel === "inputs" ? state.inputs : [];

  function changeVolume(percent: number) {
    if (output === null) return;
    showOutput({ volume: percent, muted: percent > 0 ? false : output.muted });
    control({ kind: "volume", percent });
  }
  function toggleMute() {
    if (output === null) return;
    showOutput(quiet ? { muted: false, volume: state.restore } : { ...output, muted: true });
    control({ kind: "toggleMute" });
  }
  function toggleMicrophone() {
    if (microphone === null) return;
    showMicrophone({ muted: !microphone.muted });
    control({ kind: "toggleMicrophone" });
  }
  function closePanel() { setOverlay({ layout, panel: "none" }); }
  function toggleDevices(next: "inputs" | "outputs") {
    if (!state.canSwitch) return;
    setOverlay({ layout, panel: panel === next ? "none" : next });
  }
  function toggleMixer() {
    if (view !== 0) {
      setOverlay({ layout, panel: panel === "mixer" ? "none" : "mixer" });
      return;
    }
    const next = panel === "mixer";
    setPendingCollapsed(next);
    closePanel();
    void setSoundMixerCollapsed(next).catch(() => { setPendingCollapsed(null); });
  }
  function pick(device: Device) {
    control({ kind: panel === "inputs" ? "input" : "device", id: device.id });
    closePanel();
  }
  /** Ячейка столбика: пустое место в ней раскрывает список, свои кнопки — своё. */
  function openFromCell(next: "inputs" | "outputs") {
    return (event: MouseEvent<HTMLDivElement>) => {
      if (event.target instanceof Element && event.target.closest("button")) return;
      toggleDevices(next);
    };
  }
  const volume = output === null ? "—" : `${quiet ? 0 : output.volume} %`;
  const speakerButton = (
    <SoundButton className="sound__speaker" icon={<Flip now={quiet ? <SpeakerMutedIcon /> : <SpeakerIcon />}
      next={quiet ? <SpeakerIcon /> : <SpeakerMutedIcon />} />}
      quiet={quiet && output !== null} disabled={output === null}
      title={t(quiet ? "sound.unmuteMixer" : "sound.muteMixer")} onClick={toggleMute} />
  );
  const volumeSlider = (vertical = false) => (
    <Slider value={quiet ? 0 : output?.volume ?? 0} min={0} max={100}
      disabled={output === null} vertical={vertical}
      ariaLabel={t("sound.masterVolume")} onChange={changeVolume} />
  );
  const master = (
    <div className="sound__master">
      {speakerButton}{volumeSlider()}
      <span className="sound__master-percent sound__percent tabular">{volume}</span>
    </div>
  );
  const outputControl = (
    <div className="sound__output">
      <SoundButton className="sound__output-name" icon={<DeviceIcon kind={current?.kind ?? "other"} />}
        disabled={!state.canSwitch || current === null} title={state.canSwitch
          ? t("sound.outputCycle", { name: current?.name ?? t("sound.output") })
          : current?.name ?? t("sound.noOutput")} onClick={() => control({ kind: "nextDevice" })}>
        <span className="sound__caption" title={current?.name}>{current?.short ?? t("sound.noOutput")}</span>
      </SoundButton>
      <SoundButton className="sound__small" icon={<CycleIcon />} disabled={!state.canSwitch || current === null}
        title={t("sound.nextOutput")} onClick={() => control({ kind: "nextDevice" })} />
      <SoundButton className="sound__small" icon={<ChevronIcon />} disabled={!state.canSwitch}
        expanded={panel === "outputs"} title={t("sound.showOutputs")} onClick={() => toggleDevices("outputs")} />
    </div>
  );
  const micStatus = t(microphone === null ? "sound.noMicrophone" : microphone.muted ? "sound.micOff" : "sound.micOn");
  const micHint = microphone === null ? micStatus : t(microphone.muted ? "sound.micUnmute" : "sound.micMute");
  const micButton = (
    <SoundButton className="sound__mic-toggle" icon={<Flip now={microphone?.muted ? <MicrophoneMutedIcon /> : <MicrophoneIcon />}
      next={microphone?.muted ? <MicrophoneIcon /> : <MicrophoneMutedIcon />} />}
      quiet={microphone?.muted} disabled={microphone === null} title={micHint} onClick={toggleMicrophone}>
      {view === 0 && <span className="sound__mic-copy">
        <span className="sound__mic-label">{t("sound.mic")}</span>
        <span className="sound__mic-status">{micStatus}</span>
      </span>}
    </SoundButton>
  );
  const mic = (
    <div className="sound__microphone">
      {micButton}
      <SoundButton className="sound__input-picker" icon={<ChevronIcon />} disabled={!state.canSwitch}
        expanded={panel === "inputs"} title={t("sound.showInputs")} onClick={() => toggleDevices("inputs")}>
        {view === 0 && <span className="sound__caption">{t("sound.input")}</span>}
      </SoundButton>
    </div>
  );
  const mixer = (
    <SoundButton className="sound__mixer-toggle" icon={<ChevronIcon up={panel === "mixer"} />} expanded={panel === "mixer"}
      title={t(panel === "mixer" ? "sound.hideApps" : "sound.showApps")} onClick={toggleMixer}>
      <span className="sound__caption">{t("sound.mixer")}</span>
    </SoundButton>
  );

  const panelHeading = (persistent = false) => (
    <div className="sound__panel-head">
      <span className="sound__panel-title">{t(panel === "inputs" ? "sound.showInputs" : panel === "outputs" ? "sound.showOutputs" : "sound.apps")}</span>
      <SoundButton className="sound__panel-close" icon={<ChevronIcon up={panel !== "none"} />}
        expanded={persistent ? panel !== "none" : undefined}
        title={t(panel === "none" ? "sound.showApps" : panel === "mixer" ? "sound.hideApps" : "sound.closeList")}
        onClick={panel === "mixer" || panel === "none" ? toggleMixer : closePanel}>
        {(panel === "mixer" || panel === "none") && <span>{t(panel === "none" ? "sound.show" : "sound.hide")}</span>}
      </SoundButton>
    </div>
  );
  // Прокручиваемый список столбика окно не тянет: иначе за полосу прокрутки
  // поехало бы окно, а не список
  const contents = <ul className="sound__list" data-no-drag={view === 2 || undefined}>
    {panel === "mixer" ? <>
      {state.sessions.map((track) => <TrackRow key={track.id} track={track} />)}
      {state.sessions.length === 0 && <li className="sound__empty">{t("sound.silence")}</li>}
    </> : <>
      {rows.map((device) => <DeviceRow key={device.id} device={device} input={panel === "inputs"} onPick={() => pick(device)} />)}
      {rows.length === 0 && <li className="sound__empty">{t("sound.noDevices")}</li>}
    </>}
  </ul>;
  const tabs: { id: "outputs" | "inputs" | "mixer"; icon: ReactNode; label: string }[] = [
    { id: "outputs", icon: <SpeakerIcon />, label: t("sound.panelOutput") },
    { id: "inputs", icon: <MicrophoneIcon />, label: t("sound.panelInput") },
    { id: "mixer", icon: <MixerIcon />, label: t("sound.panelMixer") },
  ];
  return (
    // Своего правого щелчка у звука нет нигде, поэтому меню виджета — по всей карточке
    <div className="widget sound-widget" data-sound-style={design} data-menu-anywhere>
      <GlassCard accent="media" flush>
        <div className="sound" data-view={view === 1 ? "compact" : view === 2 ? "vertical" : "mixer"} data-side={side}>
          <div className="sound__base">
            <div className="sound__controls">
              {view === 0 ? <>{outputControl}{master}{mic}</> : view === 1 ? <>
                <div className="sound__compact-top">{master}{mic}</div>
                <div className="sound__compact-bottom">{outputControl}{mixer}</div>
              </> : <>
                <div className="sound__vertical-volume">{volumeSlider(true)}<span className="sound__percent tabular">{volume}</span></div>
                <div className="sound__tower">
                  <div className="sound__tower-unit sound__tower-cell" data-active={panel === "outputs"}
                    data-enabled={state.canSwitch} onClick={openFromCell("outputs")}>
                    <div className="sound__tower-actions">{speakerButton}
                      <SoundButton className="sound__tower-disclosure" icon={<ChevronIcon right={!mirrored} left={mirrored} />} disabled={!state.canSwitch}
                        expanded={panel === "outputs"} title={t("sound.showOutputs")} onClick={() => toggleDevices("outputs")} />
                    </div>
                    <button type="button" className="sound__tower-device" title={state.canSwitch
                      ? t("sound.outputCycle", { name: current?.name ?? t("sound.output") })
                      : current?.name ?? t("sound.noOutput")}
                      disabled={!state.canSwitch || current === null} onClick={() => control({ kind: "nextDevice" })}>
                      {current?.short ?? t("sound.noOutput")}
                    </button>
                  </div>
                  <div className="sound__tower-unit sound__tower-cell" data-active={panel === "inputs"}
                    data-enabled={state.canSwitch} onClick={openFromCell("inputs")}>
                    <div className="sound__tower-actions">{micButton}
                      <SoundButton className="sound__tower-disclosure" icon={<ChevronIcon right={!mirrored} left={mirrored} />} disabled={!state.canSwitch}
                        expanded={panel === "inputs"} title={t("sound.showInputs")} onClick={() => toggleDevices("inputs")} />
                    </div>
                    {/* Подпись ведёт по микрофонам, а не глушит: глушит значок над ней */}
                    <button type="button" className="sound__tower-label sound__tower-label-button"
                      title={t("sound.nextInput")} disabled={!state.canSwitch || microphone === null}
                      onClick={() => control({ kind: "nextInput" })}>
                      {t("sound.mic")}
                    </button>
                  </div>
                  {/* Не `<button>`: за кнопку окно не тянется, а ячейка — треть карточки */}
                  <div role="button" tabIndex={0} className="sound__tower-unit sound__tower-mixer" data-active={panel === "mixer"}
                    aria-expanded={panel === "mixer"} title={t(panel === "mixer" ? "sound.hideApps" : "sound.showApps")}
                    aria-label={t(panel === "mixer" ? "sound.hideApps" : "sound.showApps")} onClick={toggleMixer}
                    onKeyDown={(event) => {
                      if (event.key !== "Enter" && event.key !== " ") return;
                      event.preventDefault();
                      toggleMixer();
                    }}>
                    <span className="sound__tower-actions" aria-hidden="true">
                      <span className="sound__tower-primary sound__glyph"><MixerIcon /></span>
                      <span className="sound__tower-disclosure sound__glyph"><ChevronIcon right={!mirrored} left={mirrored} /></span>
                    </span>
                    <span className="sound__tower-label">{t("sound.mixer")}</span>
                  </div>
                </div>
              </>}
            </div>
            {view === 0 && <div className="sound__apps-heading">{panelHeading(true)}</div>}
          </div>
          {panel !== "none" && <div className="sound__panel" ref={panelBox}>
            <div className="sound__panel-body">
              {view === 2 && <div className="sound__tabs" role="group" aria-label={t("sound.mixer")}>
                {tabs.map((tab) => <SoundButton key={tab.id} icon={tab.icon} title={tab.label}
                  expanded={panel === tab.id} disabled={tab.id !== "mixer" && !state.canSwitch}
                  onClick={() => setOverlay({ layout, panel: tab.id })} />)}
              </div>}
              {view === 1 && panelHeading()}
              {view === 2 && <div className="sound__side-title">
                <span>{panel === "mixer" ? t("sound.apps") : tabs.find((tab) => tab.id === panel)?.label}</span>
                <SoundButton className="sound__side-close" icon={<span aria-hidden="true">×</span>}
                  title={t("sound.closePanel")} onClick={closePanel} />
              </div>}
              {contents}
            </div>
          </div>}
        </div>
      </GlassCard>
    </div>
  );
}
