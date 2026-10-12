import { useState } from "react";
import { Button } from "../../components/Button";
import { openWidgetSettings } from "../../lib/settings";
import { useText } from "../../lib/i18n";

/**
 * Карточка пульта, пока он не подключён: одна просьба и одно поле.
 *
 * ВАЖНО: **длинных объяснений тут больше нет, и это замер, а не вкус.** Прежде
 * панель показывала вводный абзац, три адреса Redirect URI и подпись к ним.
 * Замерено во всех трёх размерах: панели нужно 250 точек там, где есть 242,
 * 303 против 232 и 357 против 252. То есть на первом запуске, ради которого
 * панель и написана, кнопка «Подключить» оказывалась за нижним краем окна, а в
 * узких размерах туда уходило и поле. Нынешней нужно 122, 141 и 178.
 *
 * Адреса переехали в настройки виджета целиком: там они и были, и там же стоит
 * то же поле Client ID. Карточка спрашивает ровно то, чего ей не хватает, а
 * «Где взять?» открывает окно с порядком действий.
 */

type Props = {
  /** Уже сохранённый Client ID — после отключения набирать его заново незачем. */
  clientId: string;
  busy: boolean;
  onConnect: (clientId: string) => void;
  reference?: boolean;
};

export function SetupPanel({ clientId, busy, onConnect, reference = false }: Props) {
  const [value, setValue] = useState(clientId);
  const known = clientId !== "";
  const t = useText();

  return (
    <div className="setup">
      <p className="setup__lead">
        {known ? t("spotify.setup.signedOut") : t(reference ? "spotify.reference.setup" : "spotify.setup.lead")}
      </p>

      {reference && <span className="setup__label">Client ID</span>}
      <input
        type="text"
        className="setup__input"
        value={value}
        placeholder="Client ID"
        spellCheck={false}
        autoComplete="off"
        aria-label={reference ? "Client ID" : undefined}
        onChange={(event) => setValue(event.target.value)}
      />

      <div className="setup__row">
        <Button busy={busy} disabled={value.trim() === ""} onClick={() => onConnect(value.trim())}
          label={reference && busy ? t("spotify.settings.waitingBrowser") : undefined}>
          {busy
            ? t("spotify.settings.waitingBrowser")
            : known
              ? t("spotify.setup.signIn")
              : t("spotify.settings.connect")}
        </Button>

        {!known && (
          <button
            type="button"
            className="setup__more"
            onClick={() => openWidgetSettings("spotify")}
          >
            {t("spotify.setup.where")}
          </button>
        )}
      </div>
    </div>
  );
}
