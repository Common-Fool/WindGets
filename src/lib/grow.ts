import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLook, type Design } from "./look";
import { useWindowReset, useWindowResized } from "./resized";

/*
 * Окно, которое растёт под раскрытую панель, например плейлисты у пульта.
 * Звук с готовой высотой каждого вида использует usePresetWindowGrowth ниже.
 *
 * Считаем от нынешней высоты, а не ставим заранее известную: окно тянется
 * мышью, как и все прочие, и готовый размер стёр бы выбранный руками.
 * Насколько выросли — помним, иначе панель, у которой за это время убыло
 * содержимое, сжала бы окно не на столько же.
 */

/*
 * Очередь изменений размера.
 *
 * ВАЖНО: **отмерять надо от высоты, которая уже поставлена.** Высоту окна
 * спрашивают у него самого, и два вызова внахлёст — быстро раскрыли и закрыли
 * список — прочтут одно и то же число: второй отмерит от высоты, которую
 * первый ещё не успел поставить. Окно после такой пары остаётся не той высоты,
 * с которой начинало, и заметно это не сразу.
 */
let queue: Promise<void> = Promise.resolve();

/*
 * Саму высоту правит Rust (`windows::grow`), а не мы. Причина одна: окно растёт
 * вниз, и у виджета, прижатого к нижнему краю, панель уезжала за экран. Чтобы
 * поднять окно ровно на недостающее, нужна рабочая область монитора — край без
 * панели задач, — а её знает только система.
 */
function resize(by: number, active: () => boolean): void {
  if (by === 0) return;

  queue = queue.then(() => active() ? invoke<void>("grow_window", { by }) : undefined).catch(() => {});
}

export type Growth = {
  /** Стоять на столько точек выше исходной высоты. */
  open: (by: number) => void;
  /** Вернуться на исходную высоту. */
  close: () => void;
};

/**
 * Рост окна под панель.
 *
 * ВАЖНО: запомненная прибавка едет за масштабом. Ползунок множит **всю** высоту
 * окна на стороне Rust (`windows::rescale`), не разбираясь, что часть её —
 * доросшая под раскрытую панель. Не умножь мы прибавку тем же числом, закрытие
 * сжало бы окно на прежнюю величину, и после каждой правки масштаба при
 * раскрытом списке окно оставалось бы выше исходного.
 *
 * Договор этот держится на том, что `rescale` именно МНОЖИТ высоту, а не ставит
 * её реестровым числом. Одно время он ставил — для нерастягиваемых окон, — и
 * ловушка была ровно тут: реестровое число стирало раскрытую панель, фронт об
 * этом не узнавал (`widget-size` оттуда не уходит), долг оставался при нём, и
 * закрытие панели вычитало его из уже уменьшенного окна. Высота упиралась в
 * единицу, и виджет становился полоской. Правя `rescale`, читать этот абзац.
 */
export function useWindowGrowth(): Growth {
  const grown = useRef(0);
  const [resetVersion, setResetVersion] = useState(0);
  const { scale, effectiveDesign } = useLook();
  const scaledAt = useRef(scale);
  const context = useRef({ design: effectiveDesign, revision: 0, alive: true });

  // Rust уже восстановил основу нового дизайна. Старую прибавку забываем,
  // не вычитая её из новой высоты; очередь прежнего владельца отменяется.
  if (context.current.design !== effectiveDesign) {
    context.current.design = effectiveDesign;
    context.current.revision += 1;
    grown.current = 0;
  }
  useEffect(() => {
    context.current.alive = true;
    return () => {
      context.current.alive = false;
      context.current.revision += 1;
    };
  }, []);

  useEffect(() => {
    if (grown.current !== 0) grown.current *= scale / scaledAt.current;
    scaledAt.current = scale;
  }, [scale]);

  /*
   * Готовый размер ставит высоту числом, мимо нашего счёта, — значит и долга
   * после него нет. Не забудь мы его здесь, закрытие панели отняло бы высоту
   * у уже нового размера.
   */
  useWindowResized(() => {
    grown.current = 0;
    context.current.revision += 1;
  });
  useWindowReset(() => {
    grown.current = 0;
    context.current.revision += 1;
    setResetVersion((version) => version + 1);
  });

  /*
   * Прибавку доводим до нужной, а не прибавляем ещё раз: раскрытая панель,
   * попросившая другой высоты (у звука их считают по числу устройств),
   * обязана оставить окно на столько же выше исходного, а не вырасти дважды.
   */
  const open = useCallback((by: number) => {
    const revision = context.current.revision;
    resize(by - grown.current, () => context.current.alive && context.current.revision === revision);
    grown.current = by;
  }, []);

  const close = useCallback(() => {
    const revision = context.current.revision;
    resize(-grown.current, () => context.current.alive && context.current.revision === revision);
    grown.current = 0;
  }, []);

  // Один и тот же объект на все перерисовки: иначе он тянул бы за собой
  // эффекты, в чьи зависимости попал, — а высоту панели считает как раз такой
  return useMemo(() => ({ open, close }), [open, close, resetVersion]);
}

/**
 * Сколько окну нужно по высоте, чтобы карточка обняла содержимое: низ
 * содержимого плюс нижнее поле тела и рамка карточки. Всё, что в окне стоит
 * вне карточки, прибавляется как есть.
 *
 * Меряется положение, а не высота: содержимое, не влезшее в карточку, всё
 * равно разложено целиком, и его низ известен, даже если его не видно.
 */
function neededHeight(content: HTMLElement): number | null {
  const card = content.closest<HTMLElement>(".card");
  const body = content.closest<HTMLElement>(".card__body");
  if (card === null || body === null) return null;

  const box = card.getBoundingClientRect();
  const bottom = content.getBoundingClientRect().bottom;
  const tail =
    parseFloat(getComputedStyle(body).paddingBottom) +
    parseFloat(getComputedStyle(card).borderBottomWidth);

  return window.innerHeight - box.height + (bottom - box.top + tail);
}

/**
 * Окно высотой ровно по содержимому — у карточки, чья высота следует из числа
 * строк, а не из реестра: у накопителей строк столько, сколько дисков
 * показано, у процессора и видеокарты — сколько рядов осталось (этап 58).
 * Реестровый размер снят под полный набор, и лишнее в него не влезет, а
 * недостающее оставит дыру.
 *
 * Меряет заново на каждой смене `key` (что показано), на каждом изменении
 * размера карточки и на каждом изменении самого содержимого: готовый размер
 * ставит свою высоту числом, и карточка тут же доводит её до своей, а ряд,
 * пришедший со снимком, растит содержимое без всякой смены размера окна. Поля
 * карточки ужимаются порогом по высоте, поэтому после первой подгонки бывает
 * вторая — и на ней всё сходится.
 *
 * `active` — мерить есть что: пока данных нет, в карточке строка ожидания, и
 * окно сжалось бы под неё в полоску.
 */
export function useContentHeight(content: RefObject<HTMLElement | null>, active: boolean, key: string, fitWidth = false): void {
  // Замер помечен дизайном, которым его сняли: запоздалый замер прежней
  // раскладки после смены дизайна Rust отбросит (`fit_window_height`)
  const { effectiveDesign: design } = useLook();
  useEffect(() => {
    const element = content.current;
    if (!active || element === null) return;

    const card = element.closest<HTMLElement>(".card");
    if (card === null) return;

    let alive = true;
    const fit = () => {
      const height = neededHeight(element);
      // Две точки — запас на округление до пикселей: на меньшем расхождении
      // окно качалось бы от своей же подгонки
      if (height === null || (!fitWidth && Math.abs(height - window.innerHeight) < 2)) return;

      queue = queue.then(() => alive ? invoke<void>("fit_window_height", { height, design }) : undefined).catch(() => {});
    };

    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(card);
    observer.observe(element);
    return () => { alive = false; observer.disconnect(); };
  }, [content, active, key, fitWidth, design]);
}

/**
 * Панель у виджета с собственной высотой каждого размера.
 *
 * Rust отмеряет её от реестровой основы, а не от прежнего долга. Поэтому
 * смена вида, размер и повторное раскрытие C не могут вычесть старую панель
 * из новой карточки. Запросы предыдущей раскладки отбрасывают обе стороны.
 */
export function usePresetWindowGrowth(view: number, size: number, design?: Design, widgetLayout?: number): Growth {
  const { scale } = useLook();
  const [resetVersion, setResetVersion] = useState(0);
  // Прежний договор звука не меняем; новые панели включают дизайн и масштаб.
  const ownScale = design === undefined ? undefined : scale;
  const layout = useRef({ view, size, design, widgetLayout, ownScale });
  const revision = useRef(0);
  const mounted = useRef(true);
  if (layout.current.view !== view || layout.current.size !== size
      || layout.current.design !== design || layout.current.widgetLayout !== widgetLayout
      || layout.current.ownScale !== ownScale) {
    layout.current = { view, size, design, widgetLayout, ownScale };
    revision.current += 1;
  }
  useWindowResized(() => { revision.current += 1; });
  useWindowReset(() => {
    revision.current += 1;
    setResetVersion((version) => version + 1);
  });
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  return useMemo(() => {
    function fit(extra: number) {
      const sentAt = revision.current;
      queue = queue.then(async () => {
        if (!mounted.current || sentAt !== revision.current) return;
        await invoke<void>("grow_window", { by: 0, preset: { view, size, extra,
          design, layout: widgetLayout, scale: ownScale } });
      }).catch(() => {});
    }
    return { open: fit, close: () => fit(0) };
  }, [view, size, design, widgetLayout, ownScale, resetVersion]);
}

export type SoundPanelSide = "left" | "right";

/**
 * Боковая панель вертикального звука. Размеры уже включают масштаб CSS:
 * ширина несёт зазор до пульта, высота меряется от верхнего края окна.
 * Нули означают закрытие. Два остальных вида продолжают расти вниз.
 *
 * Сторону выбирает Rust по рабочей области монитора. Запросы стоят в общей
 * очереди размеров; запоздавший ответ не переворачивает новую раскладку.
 */
export function useSoundSidePanel(
  view: number, size: number, scale: number, panelWidth: number, panelHeight: number,
): SoundPanelSide {
  const [side, setSide] = useState<SoundPanelSide>("right");
  const [resized, setResized] = useState(0);
  const latest = useRef(0);
  useWindowResized(() => {
    latest.current += 1;
    setResized((value) => value + 1);
  });

  useEffect(() => {
    const requestId = ++latest.current;
    if (view !== 2) return;
    const measured = panelWidth > 0 && panelHeight > 0;
    queue = queue.then(async () => {
      if (requestId !== latest.current) return;
      const reply = await invoke<{ requestId: number; side: SoundPanelSide }>("fit_sound_panel", {
        request: { view, size, scale, requestId,
          panelWidth: measured ? panelWidth : 0, panelHeight: measured ? panelHeight : 0 },
      });
      if (requestId === latest.current && reply.requestId === requestId) setSide(reply.side);
    }).catch(() => {});
    return () => { if (latest.current === requestId) latest.current += 1; };
  }, [view, size, scale, panelWidth, panelHeight, resized]);

  return side;
}
