import { useState } from "react";
import { Button } from "../../components/Button";
import { connect, disconnect, useSpotify } from "../../lib/spotify";
import { useText } from "../../lib/i18n";

/*
 * Адреса продублированы из `src-tauri/src/spotify/callback.rs`: их надо
 * показать человеку до того, как код вообще начнёт работать. Расходиться
 * им нельзя — Spotify сверяет redirect URI строкой целиком, вместе с портом.
 */
const REDIRECT_URIS = [
  "http://127.0.0.1:8888/callback",
  "http://127.0.0.1:8889/callback",
  "http://127.0.0.1:8890/callback",
];

/**
 * Привязка к учётной записи Spotify.
 *
 * Те же команды, что у пульта: подключиться можно из любого из двух мест, и
 * состояние у них общее — оно приходит одним событием из Rust.
 */
export function SpotifySection() {
  const { view } = useSpotify();
  const t = useText();
  const known = view.status === "needsAuth" ? view.clientId : "";
  const [value, setValue] = useState(known);

  const busy = view.status === "authorizing";
  const connected = view.status === "active" || view.status === "idle";

  const state = (() => {
    switch (view.status) {
      case "needsClientId":
        return t("spotify.settings.noClientId");
      case "needsAuth":
        return t("spotify.settings.notSignedIn");
      case "authorizing":
        return t("spotify.settings.waitingBrowser");
      case "error":
        return view.reason;
      default:
        return t("spotify.settings.connected");
    }
  })();

  return (
    <div className="rows">
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("settings.source.state")}</span>
          <span className="field__value">{state}</span>
        </div>
        <p className="note">{t("spotify.settings.note")}</p>
      </div>

      {connected ? (
        <div className="field" data-hint={t("spotify.settings.disconnectNote")}>
          <Button variant="outline" onClick={() => void disconnect()}>
            {t("common.disconnect")}
          </Button>
        </div>
      ) : (
        <div className="field">
          <span className="field__name">Client ID</span>
          <input
            type="text"
            className="input"
            value={value}
            placeholder={t("spotify.settings.clientIdHint")}
            data-hint={t("spotify.settings.noSecret")}
            spellCheck={false}
            autoComplete="off"
            data-no-drag
            onChange={(event) => setValue(event.target.value)}
          />

          <div className="uris" data-hint={t("spotify.settings.urisNote")}>
            <span className="field__name caps">Redirect URI</span>
            {REDIRECT_URIS.map((uri) => (
              <code key={uri} className="uris__item" data-no-drag>
                {uri}
              </code>
            ))}
          </div>

          <Button
            busy={busy}
            disabled={value.trim() === ""}
            onClick={() => void connect(value.trim())}
          >
            {t(busy ? "spotify.settings.waitingBrowser" : "spotify.settings.connect")}
          </Button>
        </div>
      )}
    </div>
  );
}
