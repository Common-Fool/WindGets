import { useEffect, useRef, useState, type RefObject } from "react";

type Measure = (box: DOMRectReadOnly) => number;

/*
 * Мерки объявлены здесь, а не по месту: стрелка, созданная заново на каждой
 * перерисовке, тянула бы за собой и наблюдателя.
 */
const heightOf: Measure = (box) => box.height;
const widthOf: Measure = (box) => box.width;

/**
 * Размер элемента в точках, обновляемый при изменении размера окна.
 *
 * До первого измерения возвращает ноль: вызывающий должен это учитывать.
 *
 * ВАЖНО: элемент ищется на каждой перерисовке, а не только при монтировании.
 * Мерить приходится и то, чего в первый заход ещё нет: список процессов
 * появляется, когда придут данные, а до тех пор на его месте стоит плашка
 * «Список собирается». Наблюдатель, повешенный один раз на пустую ссылку,
 * молчал бы потом всегда, и виджет навсегда оставался бы с одной строкой —
 * ровно это и случилось на живой проверке.
 *
 * Пересобираем только при настоящей смене элемента: сравнение ссылок дешевле
 * наблюдателя, а заходов сюда столько же, сколько перерисовок.
 */
function useSize(target: RefObject<HTMLElement | null>, measure: Measure): number {
  const [size, setSize] = useState(0);
  const watched = useRef<HTMLElement | null>(null);
  const observer = useRef<ResizeObserver | null>(null);

  useEffect(() => {
    const element = target.current;
    if (element === watched.current) return;

    observer.current?.disconnect();
    observer.current = null;
    watched.current = element;

    if (element === null) {
      setSize(0);
      return;
    }

    const next = new ResizeObserver(([entry]) => {
      if (entry !== undefined) setSize(measure(entry.contentRect));
    });

    next.observe(element);
    observer.current = next;
  });

  /*
   * Отдельным заходом: наблюдатель переживает перерисовки и гибнет с виджетом.
   *
   * ВАЖНО: вместе с ним забываем и подопечного. `StrictMode` в разработке
   * монтирует компонент дважды — размонтирует и собирает заново, — и без этой
   * строки повторный заход выше видел бы **тот же самый элемент**, считал, что
   * вешать нечего, и молча выходил. Наблюдатель после этого не собирался
   * никогда: столбики в железе оставались пустым местом, а подписи шкалы у
   * спидтеста пропадали целиком. Поймано глазами, подтверждено замером —
   * ширина ряда менялась, а число столбиков в нём оставалось нулём.
   */
  useEffect(
    () => () => {
      observer.current?.disconnect();
      observer.current = null;
      watched.current = null;
    },
    [],
  );

  return size;
}

/**
 * Высота элемента в точках.
 *
 * Нужна графику с подписями: они отбрасываются, когда налезают друг на друга,
 * а порог у этого в точках, не в процентах. Всё остальное на графике —
 * проценты и растягивающаяся система координат, и настоящий размер им незачем.
 */
export function useHeight(target: RefObject<HTMLElement | null>): number {
  return useSize(target, heightOf);
}

/**
 * Ширина элемента в точках.
 *
 * Нужна диаграмме из столбиков: их число считается по месту, а не задаётся
 * числом. Одна и та же диаграмма стоит и в узкой колонке, и во всю карточку,
 * и постоянное число столбиков в узкой давало бы штриховку.
 */
export function useWidth(target: RefObject<HTMLElement | null>): number {
  return useSize(target, widthOf);
}
