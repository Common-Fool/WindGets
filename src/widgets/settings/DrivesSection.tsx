import { useEffect, useState } from "react";
import { Check } from "../../components/Check";
import { capacityOf, disks } from "../../lib/sensors";
import { setDiskHidden, useSettings } from "../../lib/settings";
import { useTelemetry } from "../../lib/telemetry";
import { useText } from "../../lib/i18n";

/**
 * Накопители: какие показывать в карточке.
 *
 * Решение пользователя (этап 53): дисков сейчас два, но станет больше или
 * меньше — выбор нужен в настройках виджета, а не в коде. Хранятся скрытые,
 * поэтому новый диск в карточке появляется сам, а здесь — строкой с галочкой.
 *
 * Список берётся из живой телеметрии, как у раздела «Данные»: имён дисков
 * больше взять неоткуда. Выключен модуль — датчики спят, и список пуст; тогда
 * остаются только скрытые, чтобы их можно было вернуть.
 *
 * Последнюю показанную галочку снять нельзя: пустая карточка на обоях ничего
 * не говорит, а спрятать её целиком можно галочкой «Вкл.».
 */
export function DrivesSection() {
  const t = useText();
  const telemetry = useTelemetry();
  const settings = useSettings();

  /*
   * Снятые, о которых Rust ещё не ответил снимком. Без них две галочки,
   * снятые быстрее рассылки, обошли бы защиту последнего диска: вторая ещё
   * считала бы первую показанной.
   */
  const [pending, setPending] = useState<string[]>([]);
  useEffect(() => {
    setPending((now) => now.filter((name) => !settings.hiddenDisks.includes(name)));
  }, [settings.hiddenDisks]);
  const hiddenDisks = [...settings.hiddenDisks, ...pending.filter((name) => !settings.hiddenDisks.includes(name))];

  const toggle = (name: string, show: boolean) => {
    setPending((now) => (show ? now.filter((entry) => entry !== name) : [...now, name]));
    setDiskHidden(name, !show);
  };

  const present = disks(telemetry).map((disk) => disk.name);
  // Скрытые, которых сейчас нет в выхлопе, тоже стоят в списке — иначе
  // вынутый и вставленный обратно диск вернуть было бы нечем
  const names = [...new Set(present), ...hiddenDisks.filter((name) => !present.includes(name))];
  const shown = present.filter((name) => !hiddenDisks.includes(name));
  // Одинаковые модели делят имя и снимаются одной галочкой: «последняя»
  // считается по именам, иначе два одинаковых диска давали две показанные
  // строки, и снятие одной прятало оба — карточка оставалась пустой
  const shownNames = new Set(shown);

  return (
    <div className="rows">
      <div className="field">
        {/* Подсказка на заголовке: над галочками дисков она мешала бы */}
        <div className="field__head" data-hint={t("disks.settings.hint")}>
          <span className="field__name">{t("disks.settings.title")}</span>
          <span className="field__value tabular">
            {shown.length} / {present.length}
          </span>
        </div>

        {present.length === 0 && <p className="note">{t("disks.settings.waiting")}</p>}

        {names.length > 0 && (
          <div className="rows rows--tight">
            {names.map((name) => {
              const visible = present.includes(name) && !hiddenDisks.includes(name);
              const last = visible && shownNames.size === 1;
              const capacity = capacityOf(name);
              return (
                <label key={name} className="switch" data-hint={last ? t("disks.settings.last") : undefined}>
                  <Check
                    checked={!hiddenDisks.includes(name)}
                    label={name}
                    quiet={last}
                    disabled={last}
                    onChange={(next) => toggle(name, next)}
                  />
                  <span className="switch__text">
                    <span className="switch__name">{name}</span>
                    <span className="switch__hint">
                      {present.includes(name) ? (capacity ?? "") : t("disks.settings.absent")}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
