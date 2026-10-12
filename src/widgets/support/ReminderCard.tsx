import { invoke } from "@tauri-apps/api/core";
import { Button } from "../../components/Button";
import { GlassCard } from "../../components/GlassCard";
import { useText } from "../../lib/i18n";
import "./reminder.css";

/**
 * Карточка напоминания о донатах: «Нравится Windgets?» и три кнопки.
 *
 * Окно собирает и разбирает Rust (`support_reminder.rs`) — только на время
 * показа, без фокуса и не во время игры. Сама карточка лишь передаёт ответ:
 *
 * - «Поддержать» — окно настроек на разделе «Поддержать»;
 * - «Позже» — закрыть, следующий раз через обычный срок;
 * - «Напоминать реже» — дальше не чаще раза в полгода. Полного выключения
 *   нет — слово пользователя; вернуть обычный срок можно галочкой в
 *   «Поддержать».
 */
type Answer = "support" | "later" | "rare";

function answer(value: Answer): void {
  void invoke("support_reminder_answer", { answer: value }).catch(() => {});
}

export function ReminderCard() {
  const t = useText();

  return (
    <div className="widget">
      <GlassCard accent="neutral">
        <div className="reminder">
          <p className="reminder__title">{t("reminder.title")}</p>
          <p className="reminder__text">{t("reminder.text")}</p>
          <div className="reminder__actions">
            <Button variant="outline" onClick={() => answer("support")}>
              {t("reminder.support")}
            </Button>
            <Button variant="outline" onClick={() => answer("later")}>
              {t("reminder.later")}
            </Button>
            <button type="button" className="reminder__rare" onClick={() => answer("rare")}>
              {t("reminder.rare")}
            </button>
          </div>
        </div>
      </GlassCard>
    </div>
  );
}
