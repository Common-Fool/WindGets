import { useEffect, useState } from "react";
import { fetchPlaylists, type Playlist } from "../../lib/spotify";
import { SpotifyMark } from "./icons";
import { useText } from "../../lib/i18n";

type Props = {
  onPlay: (uri: string) => void;
  /**
   * Высота панели в точках — ровно та, на которую выросло окно.
   *
   * ВАЖНО: **число приходит снаружи, а не стоит в CSS.** Панель и пульт иначе
   * делят прибавку как два растяжимых блока: пульт сперва сжимается под новую
   * панель, потом окно дорастает и он частично отыгрывает. Раскладка при этом
   * дёргается дважды, а обложка при раскрытых плейлистах оказывается другого
   * размера, чем при закрытых.
   */
  height: number;
};

export function PlaylistPanel({ onPlay, height }: Props) {
  const t = useText();
  const [items, setItems] = useState<Playlist[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Список запрашиваем один раз на открытие: он меняется куда реже,
    // чем состояние плеера, и держать его в опросе незачем
    let alive = true;

    fetchPlaylists()
      .then((list) => {
        if (alive) setItems(list);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });

    return () => {
      alive = false;
    };
  }, []);

  if (error !== null) {
    return (
      <div className="playlists__note playlists__note--danger" style={{ height }}>
        {t("spotify.playlists.failed", { error })}
      </div>
    );
  }

  if (items === null) {
    return (
      <div className="playlists__note" style={{ height }}>
        {t("common.loading")}
      </div>
    );
  }

  if (items.length === 0) {
    return (
      <div className="playlists__note" style={{ height }}>
        {t("spotify.playlists.none")}
      </div>
    );
  }

  return (
    <ul className="playlists" style={{ height }}>
      {items.map((playlist) => (
        <li key={playlist.id}>
          <button
            type="button"
            className="playlists__item"
            onClick={() => onPlay(playlist.uri)}
            title={t("spotify.playlists.play", { name: playlist.name })}
          >
            {playlist.coverUrl === null ? (
              <span className="playlists__cover playlists__cover--empty">
                <SpotifyMark />
              </span>
            ) : (
              <img
                className="playlists__cover"
                src={playlist.coverUrl}
                alt=""
                draggable={false}
                referrerPolicy="no-referrer"
              />
            )}
            <span className="playlists__name">{playlist.name}</span>
            <span className="playlists__count tabular">{playlist.tracks}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
