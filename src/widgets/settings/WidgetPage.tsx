import { useState, type ComponentType } from "react";
import {
  setWidgetCover,
  setWidgetLayout,
  setWidgetSize,
  setWidgetView,
  useSettings,
  type WidgetRow,
} from "../../lib/settings";
import {
  commitLook,
  MAX_SCALE,
  MIN_OPACITY,
  MIN_SCALE,
  previewWidgetOpacity,
  previewWidgetScale,
  useLook,
} from "../../lib/look";
import { Check } from "../../components/Check";
import { useText } from "../../lib/i18n";
import { Slider } from "../../components/Slider";
import { ConverterSection } from "./ConverterSection";
import { DownloaderSection } from "./DownloaderSection";
import { DriverField } from "./DriverField";
import { DrivesSection } from "./DrivesSection";
import { MailAccountSection, MailBoxesSection, MailPreviewField } from "./MailSection";
import { useMail } from "../../lib/mail";
import { MediaSection } from "./MediaSection";
import { MetricsSection } from "./MetricsSection";
import { SpotifySection } from "./SpotifySection";
import { TotpSection } from "./TotpSection";
import { WeatherSection } from "./WeatherSection";
import { SpeedtestSource } from "./DataSource";

/*
 * Страница одного виджета: вид, обложка, готовый размер, своя плотность фона и
 * собственный раздел, если он есть.
 *
 * Своим окном это было один этап и слито в общие настройки: настройки виджета
 * — раздел общих, а не вторая сущность. Открывают её двумя путями — шестерёнкой
 * в списке виджетов и пунктом «Настройки…» в меню самого виджета, — и оба
 * ведут в одно окно, на эту страницу.
 *
 * Разделы «Погода», «Коды 2FA», «Медиа», Spotify, почты, конвертера и
 * загрузчика стоят тут же: они и есть настройки конкретного виджета.
 * Подключения Spotify и почты ненадолго выносились общим разделом
 * «Подключения» и вернулись сюда с живой проверки (этап 52, слово
 * пользователя): искать учётную запись виджета естественнее у самого виджета.
 */

/**
 * Разделы, которые есть только у своего виджета: подключение, город,
 * записи, программы. Стоят первыми и отчёркнуты от общих настроек карточки.
 *
 * Раздел получает `id` своего виджета: состав карточки (этап 58) один на
 * процессор и видеокарту, и иначе ему было бы не узнать, чей он.
 */
const OWN_SECTIONS: Record<string, ComponentType<{ id: string }>> = {
  converter: ConverterSection,
  cpu: CpuSettings,
  disks: DrivesSection,
  downloader: DownloaderSection,
  gpu: MetricsSection,
  mail: MailBoxesSection,
  media: MediaSection,
  speedtest: SpeedtestSource,
  spotify: SpotifySection,
  totp: TotpSection,
  weather: WeatherSection,
};

/**
 * Процессор: первым — драйвер PawnIO, без которого карточка заперта, ниже
 * состав карточки и датчики платы.
 */
function CpuSettings({ id }: { id: string }) {
  return (
    <>
      <DriverField />
      <MetricsSection id={id} />
    </>
  );
}

/**
 * Своё у виджета, но про вид карточки, а не про подключение: стоит под чертой,
 * рядом с видом и размером. Строка содержимого у почты — вид письма в
 * карточке, и сохранением учётной записи она не пишется.
 */
const OWN_LOOK: Record<string, ComponentType> = {
  mail: MailPreviewField,
};

/**
 * Страница одного виджета.
 *
 * Дороги назад над заголовком нет: виджет выбран в дереве колонки, и
 * «Виджеты» стоят там же строкой выше.
 *
 * `page` — страница внутри виджета: у почты это ящик по ключу или `new`
 * (этап 53). Вид карточки у ящика не свой, поэтому на его странице только
 * учётная запись.
 */
export function WidgetPage({ id, page }: { id: string; page?: string }) {
  if (id === "mail" && page !== undefined) return <MailBoxPage id={page} />;
  return <WholeWidgetPage id={id} />;
}

/** Страница одного ящика почты: заголовок — адрес, под ним форма. */
function MailBoxPage({ id }: { id: string }) {
  const t = useText();
  const box = useMail()?.accounts.find((one) => one.id === id);
  const title = box !== undefined && box.address !== "" ? box.address : t("mail.settings.newBox");

  return (
    <>
      <h2 className="settings__heading">{title}</h2>
      {/* Ключом — чтобы форма другого ящика не начинала с полей прежнего */}
      <MailAccountSection key={id} id={id} />
    </>
  );
}

function WholeWidgetPage({ id }: { id: string }) {
  const row = useSettings().widgets.find((widget) => widget.id === id);
  const Section = OWN_SECTIONS[id];
  const Look = OWN_LOOK[id];
  const t = useText();
  const look = useLook();

  return (
    <>
      <h2 className="settings__heading caps">{row?.title ?? t("page.widget")}</h2>

      {row === undefined ? (
        <p className="note">{t("page.notFound")}</p>
      ) : (
        <div className="rows">
          {Section !== undefined && (
            <>
              <Section id={id} />
              <hr className="divider" />
            </>
          )}

          {look.effectiveDesign === "reference" && (
            <div className="settings__card-caption" data-hint={t("look.designNote")}>
              <h3>{t("settings.card.parameters")}</h3>
              <span>{t(look.design === "reference" ? "design.gpt" : "design.claude")}</span>
            </div>
          )}
          {Look !== undefined && <Look />}
          <Layouts row={row} />
          <Views row={row} />
          <Covers row={row} />
          <Sizes row={row} />
          {/* Ключом — чтобы при переходе к другому виджету ползунки начинали
              с его собственных значений, а не с чужих */}
          <OwnLook key={row.id} row={row} />
        </div>
      )}
    </>
  );
}

/**
 * Чем карточка себя рисует. Пункта нет вовсе, если вид один.
 *
 * Здесь и в меню виджета — единственные два места, где вид меняют: щелчком по
 * карточке он не листается с этапа 53.
 */
function Layouts({ row }: { row: WidgetRow }) {
  const t = useText();
  if ((row.layouts?.length ?? 0) < 2) return null;
  return <div className="field">
    <div className="field__head"><span className="field__name">{t("page.layout")}</span></div>
    <div className="choices">{row.layouts.map((name, index) => <button key={name} type="button" className="choice"
      aria-pressed={index === row.layout} onClick={() => setWidgetLayout(row.id, index)}>{name}</button>)}</div>
  </div>;
}

function Views({ row }: { row: WidgetRow }) {
  const t = useText();
  if (row.views.length < 2) return null;

  return (
    <div className="field">
      <div className="field__head">
        <span className="field__name">{t("page.view")}</span>
      </div>
      <div className="choices">
        {row.views.map((name, index) => (
          <button
            key={name}
            type="button"
            className="choice"
            aria-pressed={index === row.view}
            onClick={() => setWidgetView(row.id, index)}
          >
            {name}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * Чем в карточке служит обложка.
 *
 * Третья ось помимо вида и размера. Отдельным разделом, а не пунктами вида:
 * обложка сочетается с любой раскладкой и любым размером, и слитые в один
 * список они дали бы полсотни пунктов.
 */
function Covers({ row }: { row: WidgetRow }) {
  const t = useText();
  if (row.covers.length < 2) return null;

  return (
    <div className="field" data-hint={t("page.coverNote")}>
      <div className="field__head">
        <span className="field__name">{t("page.cover")}</span>
      </div>
      <div className="choices">
        {row.covers.map((name, index) => (
          <button
            key={name}
            type="button"
            className="choice"
            aria-pressed={index === row.cover}
            onClick={() => setWidgetCover(row.id, index)}
          >
            {name}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * Готовый размер окна.
 *
 * Свободное растягивание может дополнять пресеты. Когда пресет один,
 * вместо пустого списка поясняем, что окно можно растянуть мышью.
 */
function Sizes({ row }: { row: WidgetRow }) {
  const t = useText();
  if (["notes", "clipboard", "todo", "converter"].includes(row.id)) return null;
  if (row.flexible && row.sizes.length < 2) {
    return (
      <div className="field">
        <div className="field__head">
          <span className="field__name">{t("page.size")}</span>
        </div>
        <p className="note">{t("page.sizeFlexible")}</p>
      </div>
    );
  }

  if (row.sizes.length < 2) return null;

  return (
    <div className="field" data-hint={t(row.flexible ? "page.sizeFlexible" : "page.sizeNote")}>
      <div className="field__head">
        <span className="field__name">{t("page.size")}</span>
      </div>
      <div className="choices">
        {row.sizes.map((name, index) => (
          <button
            key={name}
            type="button"
            className="choice"
            aria-pressed={(row.sizeIndices?.[index] ?? index) === row.size}
            onClick={() => setWidgetSize(row.id, row.sizeIndices?.[index] ?? index)}
          >
            {name}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * Своя плотность фона и свой масштаб.
 *
 * Оба устроены одинаково, поэтому и компонент один: `null` значит «как у
 * всех», и пока галочка стоит, ползунок показывает общее число и идёт за
 * общим — своего у виджета в этот момент нет вовсе.
 *
 * ВАЖНО: **галочка снимается сама, стоит тронуть ползунок.** Иначе движение
 * ничего бы не делало до отдельного нажатия, и это читалось бы как сломанный
 * ползунок, а не как запрет.
 */
function Own({
  id,
  name,
  hint,
  low,
  high,
  min,
  max,
  step,
  own,
  common,
  preview,
}: {
  id: string;
  name: string;
  hint: string;
  low: string;
  high: string;
  min: number;
  max: number;
  step?: number;
  /** Своё число или `null` — «как у всех». */
  own: number | null;
  /** Общее число: его показываем, пока своего нет. */
  common: number;
  preview: (id: string, value: number | null) => void;
}) {
  const t = useText();
  const [custom, setCustom] = useState(own !== null);
  const [draft, setDraft] = useState(own ?? common);

  const percent = Math.round((custom ? draft : common) * 100);

  const follow = (next: boolean) => {
    setCustom(!next);
    // Своё число заводим с того, что сейчас показано, а не с чёрновика: пока
    // галочка стояла, ползунок шёл за общим, и запомненное с прошлого раза
    // значение прыгнуло бы под рукой
    if (!next) setDraft(common);
    preview(id, next ? null : common);
    commitLook();
  };

  return (
    <div className="field">
      <div className="field__head">
        <span className="field__name">{name}</span>
        <span className="field__value tabular">{percent} %</span>
      </div>

      <Slider
        value={percent}
        min={min * 100}
        max={max * 100}
        step={step}
        onChange={(next) => {
          const value = next / 100;
          setDraft(value);
          setCustom(true);
          preview(id, value);
        }}
        onCommit={commitLook}
      />
      <div className="field__scale">
        <span>{low}</span>
        <span>{high}</span>
      </div>

      <label className="switch" data-hint={hint}>
        <Check checked={!custom} label={t("page.likeAllOf", { name })} quiet onChange={follow} />
        <span className="switch__text">
          <span className="switch__name">{t("page.likeAll")}</span>
        </span>
      </label>
    </div>
  );
}

/** Оба своих числа виджета: плотность фона и масштаб. */
function OwnLook({ row }: { row: WidgetRow }) {
  const look = useLook();
  const t = useText();

  return (
    <>
      <Own
        id={row.id}
        name={t("page.opacity")}
        hint={t("page.opacityHint")}
        low={t("look.opacityLow")}
        high={t("look.opacityHigh")}
        min={MIN_OPACITY}
        max={1}
        own={row.cardOpacity}
        common={look.cardOpacity}
        preview={previewWidgetOpacity}
      />

      <Own
        id={row.id}
        name={t("look.scale")}
        hint={t("page.scaleHint")}
        low={`${Math.round(MIN_SCALE * 100)} %`}
        high={`${Math.round(MAX_SCALE * 100)} %`}
        min={MIN_SCALE}
        max={MAX_SCALE}
        step={5}
        own={row.scale}
        common={look.scale}
        preview={previewWidgetScale}
      />
    </>
  );
}
