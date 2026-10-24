import { useEffect, useRef, useState } from "react";
import "./Marquee.css";

/**
 * Строка, которая раз в несколько секунд пробегает целиком и возвращается.
 *
 * Нужна там, где имя приходит извне и его длину не выбирают: название трека
 * бывает вдвое длиннее карточки, а эллипсис обрезает его ровно на том слове,
 * ради которого на карточку и смотрят.
 *
 * ВАЖНО: **бежит только то, что не поместилось.** Влезающая строка стоит
 * неподвижно и анимации не получает вовсе — иначе полтора десятка окон на
 * обоях круглые сутки гоняли бы композитор ради текста, который и так виден.
 *
 * Покой длиннее хода: строка стоит началом вперёд, изредка показывает хвост и
 * возвращается. Обратное — бесконечная карусель — на обоях читается как
 * бегущая строка вокзала и мешает всему, что рядом.
 */
type Props = {
  text: string;
  /** Класс на саму строку: кегль и цвет берутся с места, а не отсюда. */
  className?: string;
  /** Подсказка при наведении. Обычно тот же текст целиком. */
  title?: string;
  /**
   * Бежать только пока это `true` — например, пока мышь над карточкой. Без
   * него строка изредка пробегает сама. Выключенная, строка стоит началом
   * вперёд и показывает то, что влезает.
   */
  active?: boolean;
  /**
   * Пока строка стоит, показывать только целые слова, которые влезают, — без
   * обрубка на полуслове. Бежит она всё равно целиком.
   */
  wholeWords?: boolean;
};

/**
 * Сколько точек в секунду проходит строка.
 *
 * Медленно намеренно: глаз читает бегущий текст примерно на этой скорости, а
 * быстрее строка превращается в мельтешение на краю зрения.
 */
const SPEED = 50;

/** Ход в одну сторону занимает 30 % цикла — остальное покой. */
const TRAVEL_SHARE = 0.3;

/** Короче этого цикл не бывает: у коротких хвостов покой важнее хода. */
const MIN_CYCLE = 6;

/** Ниже этого не бежим: пара точек разницы — это округление, а не хвост. */
const SLACK = 2;

/** Холст для замеров ширины текста: один на модуль, в DOM не попадает. */
let ruler: CanvasRenderingContext2D | null = null;

/** Ширина строки шрифтом элемента. */
function textWidth(element: HTMLElement, text: string): number {
  ruler ??= document.createElement("canvas").getContext("2d");
  if (ruler === null) return 0;
  ruler.font = getComputedStyle(element).font;
  return ruler.measureText(text).width;
}

/** Самое длинное начало из целых слов, которое влезает в `room`; хоть одно слово — всегда. */
function wordsThatFit(element: HTMLElement, text: string, room: number): string {
  const words = text.split(/\s+/).filter(Boolean);
  let fitted = words[0] ?? "";
  for (let count = 2; count <= words.length; count++) {
    const candidate = words.slice(0, count).join(" ");
    if (textWidth(element, candidate) > room) break;
    fitted = candidate;
  }
  return fitted;
}

export function Marquee({ text, className, title, active, wholeWords = false }: Props) {
  const host = useRef<HTMLDivElement | null>(null);
  const line = useRef<HTMLSpanElement | null>(null);
  const [run, setRun] = useState(0);
  const [fitted, setFitted] = useState(text);
  // Влезает ли укороченная строка: одно длинное слово не влезает и тогда
  // стоит началом слева, а не обрезается по центру с обеих сторон
  const [fits, setFits] = useState(true);

  useEffect(() => {
    const frame = host.current;
    const inner = line.current;
    if (frame === null || inner === null) return;

    const measure = () => {
      // По словам строка в покое короче полной, и `scrollWidth` мерил бы её;
      // ход считается от полной, поэтому она меряется холстом
      const full = wholeWords ? textWidth(inner, text) : inner.scrollWidth;
      const over = full - frame.clientWidth;
      setRun(over > SLACK ? over : 0);
      if (wholeWords) {
        const words = over > SLACK ? wordsThatFit(inner, text, frame.clientWidth) : text;
        setFitted(words);
        setFits(textWidth(inner, words) <= frame.clientWidth + SLACK);
      }
    };

    measure();

    /*
     * ВАЖНО: **наблюдатель обязан пережить повторное монтирование.**
     * `StrictMode` в разработке собирает компонент дважды, и наблюдатель тут
     * заводится свой на каждый заход — поэтому `disconnect` в уборке ничего
     * чужого не гасит. Общий наблюдатель на модуль этого бы не пережил: второй
     * заход увидел бы тот же элемент уже подопечным и не подписался бы вовсе.
     */
    const watcher = new ResizeObserver(measure);
    watcher.observe(frame);
    return () => watcher.disconnect();
  }, [text, wholeWords]);

  const cycle = Math.max(MIN_CYCLE, run / SPEED / TRAVEL_SHARE);
  const [away, setAway] = useState(false);

  /*
   * ВАЖНО: **в покое анимации нет вовсе — ход идёт переходом, а паузы
   * таймером.** Прежде цикл был одной бесконечной анимацией, и покой в ней —
   * это кадры с неподвижной строкой: композитор рисовал их шестьдесят раз в
   * секунду все семьдесят процентов цикла, пока трек играл часами. Теперь
   * между ходами строка просто стоит, и окно не рисует ничего.
   *
   * Доли прежние: покой 20 %, ход 30, у хвоста 15, ход назад 30, покой 5.
   */
  useEffect(() => {
    setAway(false);
    if (run === 0 || active === false || matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    const travel = cycle * TRAVEL_SHARE * 1000;
    // Таймер всегда один: следующий ставится из предыдущего
    let timer = 0;
    const at = (delay: number, step: () => void) => {
      timer = window.setTimeout(step, delay);
    };
    // По наведению ход начинается почти сразу: строку позвали, а не ждут
    let rest = active === undefined ? 0.2 : 0.05;
    const loop = () => {
      at(cycle * rest * 1000, () => {
        rest = 0.2;
        setAway(true);
        at(travel + cycle * 0.15 * 1000, () => {
          setAway(false);
          at(travel + cycle * 0.05 * 1000, loop);
        });
      });
    };
    loop();
    return () => window.clearTimeout(timer);
    // Текст тоже: новое название того же переполнения иначе вставало бы
    // сразу хвостом — ход прежнего продолжался бы с середины
  }, [text, run, cycle, active]);

  // В покое по словам стоит укороченная строка по центру, бежит — полная
  const resting = wholeWords && run > 0 && active === false;
  const shown = resting ? fitted : text;

  const style =
    run > 0
      ? ({
          "--marquee-run": `${run}px`,
          "--marquee-time": `${(cycle * TRAVEL_SHARE).toFixed(2)}s`,
        } as React.CSSProperties)
      : undefined;

  const frame = ["marquee"];
  if (run > 0 && (!resting || !fits)) frame.push("marquee--runs");
  if (className !== undefined) frame.push(className);

  return (
    <div ref={host} className={frame.join(" ")} title={title ?? text} style={style}>
      <span
        ref={line}
        className={run > 0 ? "marquee__line marquee__line--runs" : "marquee__line"}
        data-away={away && run > 0 ? "true" : undefined}
      >
        {shown}
      </span>
    </div>
  );
}
