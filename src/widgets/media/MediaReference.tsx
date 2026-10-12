import { Marquee } from "../../components/Marquee";
import { useText } from "../../lib/i18n";
import { clock, control, pickSession, titleOf, type Track } from "../../lib/media";
import type { SeekDrag } from "../../lib/seek";
import { MediaGlyph } from "./media-icons";

type PlayerProps = {
  track: Track;
  size: number;
  layout: number;
  cover: string | null;
  playing: boolean;
  position: number;
  others: number;
  opened: boolean;
  onSwitch: () => void;
  onOpen: () => void;
  onToggle: () => void;
  seek: SeekDrag;
};

/** Только разметка GPT: команды и рабочее состояние принадлежат общему Player. */
export function ReferencePlayer({
  track, size, layout, cover, playing, position, others, opened,
  onSwitch, onOpen, onToggle, seek,
}: PlayerProps) {
  const t = useText();
  const source = <ReferenceSource track={track} others={others} opened={opened}
    onSwitch={onSwitch} onOpen={onOpen} />;

  return (
    <div className="media-reference__base">
      <ReferenceCover url={cover} />
      <div className="media-reference__meta">
        {layout === 0 && source}
        <Marquee className="media-reference__song" text={track.song === "" ? t("media.unknownTrack") : track.song} />
        {track.artist !== "" && <Marquee className="media-reference__artist" text={track.artist} />}
      </div>
      <div className="media-reference__controls">
        <button type="button" className="media-reference__button"
          aria-label={t("transport.previous")} title={t("transport.previous")}
          disabled={!track.canPrevious} onClick={() => void control(track.source, "previous")}>
          <MediaGlyph name="previous" />
        </button>
        <button type="button" className="media-reference__button media-reference__button--play"
          aria-label={playing ? t("transport.pause") : t("transport.play")}
          title={playing ? t("transport.pause") : t("transport.play")}
          disabled={!track.canPlay} onClick={onToggle}>
          <MediaGlyph name={playing ? "pause" : "play"} />
        </button>
        <button type="button" className="media-reference__button"
          aria-label={t("transport.next")} title={t("transport.next")}
          disabled={!track.canNext} onClick={() => void control(track.source, "next")}>
          <MediaGlyph name="next" />
        </button>
      </div>
      {size !== 3 && <ReferenceProgress track={track} position={position}
        showTimes={size < 2} seek={seek} />}
      {layout === 1 && source}
    </div>
  );
}

export function ReferenceIdle() {
  const t = useText();
  return (
    <div className="media-reference__base" data-idle="true">
      <ReferenceCover url={null} />
      <div className="media-reference__meta">
        <span className="media-reference__song">{t("media.idle")}</span>
        <span className="media-reference__artist">{t("media.idleHint")}</span>
      </div>
    </div>
  );
}

function ReferenceCover({ url }: { url: string | null }) {
  return (
    <div className="media-reference__art">
      {url === null ? <MediaGlyph name="note" />
        : <img src={url} alt="" draggable={false} />}
    </div>
  );
}

function ReferenceSource({
  track, others, opened, onSwitch, onOpen,
}: {
  track: Track; others: number; opened: boolean;
  onSwitch: () => void; onOpen: () => void;
}) {
  const t = useText();
  const name = track.title === "" ? track.source : track.title;
  return (
    <div className="media-reference__source">
      {others < 1 ? <span className="media-reference__source-name" title={name}>{name}</span>
        : <button type="button" className="media-reference__source-cycle"
          title={t.count("media.moreSources", others)} onClick={onSwitch}>
          <span className="media-reference__source-name">{name}</span>
          <span className="media-reference__more">+{others}</span>
        </button>}
      <button type="button" className="media-reference__source-expand"
        aria-label={t("media.settings.sources")} title={t("media.settings.sources")}
        aria-expanded={opened} aria-controls="media-session-panel"
        disabled={others < 1} onClick={onOpen}>
        <MediaGlyph name={opened ? "up" : "down"} />
      </button>
    </div>
  );
}

function ReferenceProgress({
  track, position, showTimes, seek,
}: {
  track: Track; position: number; showTimes: boolean; seek: SeekDrag;
}) {
  const t = useText();
  const known = track.duration > 0;
  const canSeek = known && track.canSeek;
  return (
    <div className="media-reference__progress" data-times={showTimes} data-known={known}>
      {showTimes && <span className="media-reference__time tabular">{clock(seek.chosen === null ? position : seek.chosen * track.duration)}</span>}
      <div className="media-reference__bar" data-seek={canSeek} data-no-drag=""
        {...seek.handlers} role="presentation" title={canSeek ? t("media.seek") : undefined}>
        <div className="media-reference__fill" style={{ width: (seek.shown * 100).toFixed(2) + "%" }} />
        {canSeek && <span className="media-reference__thumb" style={{ left: (seek.shown * 100).toFixed(2) + "%" }} />}
      </div>
      {showTimes && <span className="media-reference__time media-reference__time--end tabular">{clock(track.duration)}</span>}
    </div>
  );
}

/** Выбор из уже разрешённых sessions; галочки разрешения остаются в настройках. */
export function ReferenceSessions({ sessions, current }: { sessions: Track[]; current: string }) {
  const t = useText();
  return (
    <div className="media-reference__panel" id="media-session-panel">
      <div className="media-reference__panel-head">
        <span>{t("media.settings.sources")}</span>
        <span className="tabular">{sessions.length}</span>
      </div>
      <ul className="media-reference__sessions">
        {sessions.map((session) => {
          const selected = session.source === current;
          const name = session.title === "" ? session.source : session.title;
          return (
            <li key={session.source}>
              <button type="button" className="media-reference__session"
                aria-pressed={selected} onClick={() => void pickSession(session.source)}>
                <MediaGlyph name="note" />
                <span className="media-reference__session-meta">
                  <span className="media-reference__session-name" title={name}>{name}</span>
                  <span className="media-reference__session-track" title={titleOf(session)}>{titleOf(session)}</span>
                </span>
                <MediaGlyph name={selected ? "check" : session.playing ? "play" : "pause"} />
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
