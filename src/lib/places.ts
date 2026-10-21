import { useCallback, useEffect, useRef, useState } from "react";
import { useDebounced } from "./debounce";
import { searchPlaces, setWeatherPlace, type Place } from "./settings";

/*
 * Поиск города по названию — один на два места: карточку погоды на первом
 * запуске и раздел «Погода» в настройках виджета.
 *
 * Вынесен сюда ровно поэтому. Правила у поиска свои — задержка после последней
 * буквы, порог в три символа, выброшенный ответ на устаревший запрос, — и двум
 * их копиям разойтись было бы вопросом времени: карточка чинилась бы отдельно
 * от настроек, а замечали бы это через раз.
 */

export type PlaceSearch =
  | { status: "idle" }
  | { status: "busy" }
  | { status: "done"; hits: Place[] }
  | { status: "failed"; reason: string };

/**
 * Сколько молчит поле, прежде чем запрос уйдёт.
 *
 * Секунда покоя и есть ответ на страх перед длинными названиями: пока печатают
 * «Екатеринбург», в сеть не уходит ничего.
 */
const SEARCH_DELAY_MS = 1000;

/** Короче трёх букв сами не ищем: находки будут случайными. */
const MIN_QUERY_LENGTH = 3;

/**
 * Но иероглифы — не буквы: у китайских городов имя из двух знаков («北京»,
 * «上海») и есть всё название, и с порогом в три их было не найти никак. Тот
 * же предел в два знака у Enter: им просят искать именно то, что набрано, —
 * ровно столько пускает и Rust (`api::MIN_QUERY`).
 */
const MIN_QUERY_SHORT = 2;
const IDEOGRAPHS = /[぀-ヿ㐀-鿿가-힯]/;

function askable(text: string, enter: boolean): boolean {
  const least = enter || IDEOGRAPHS.test(text) ? MIN_QUERY_SHORT : MIN_QUERY_LENGTH;
  return [...text].length >= least;
}

export type PlaceSearchControls = {
  query: string;
  search: PlaceSearch;
  /** Ввод в поле. Пустая строка гасит список находок. */
  type: (next: string) => void;
  /** Enter: ищем сразу, не дожидаясь задержки. */
  now: () => void;
  /** Город выбран: запоминаем его и убираем список. */
  choose: (place: Place) => void;
};

export function usePlaceSearch(): PlaceSearchControls {
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState<PlaceSearch>({ status: "idle" });

  const queryRef = useRef(query);
  queryRef.current = query;

  const lastAskedRef = useRef("");
  const debounced = useDebounced(query, SEARCH_DELAY_MS);

  const ask = useCallback((wanted: string, force = false) => {
    if (!askable(wanted, force)) return;
    if (!force && wanted === lastAskedRef.current) return;

    lastAskedRef.current = wanted;
    setSearch({ status: "busy" });

    /*
     * Ответ устарел. Если новее этого запроса не уходило, ждать больше нечего:
     * поле вернули к тому же тексту или укоротили, задержка второй раз не
     * сработает, и «Ищем…» висело бы навсегда. Спрашиваем то, что в поле
     * сейчас, или гасим поиск.
     */
    const outdated = () => {
      if (wanted !== lastAskedRef.current) return;
      const current = queryRef.current.trim();
      if (askable(current, false)) {
        ask(current, true);
      } else {
        lastAskedRef.current = "";
        setSearch({ status: "idle" });
      }
    };

    searchPlaces(wanted)
      .then((hits) => {
        // ВАЖНО: ответ на устаревший запрос выбрасываем. Запросы уходят
        // внахлёст, и ответ на «Гра» приходит после ответа на «Грац» —
        // показанный, он подменил бы находки на чужие
        if (queryRef.current.trim() !== wanted) return outdated();
        setSearch({ status: "done", hits });
      })
      .catch((error: unknown) => {
        if (queryRef.current.trim() !== wanted) return outdated();
        setSearch({ status: "failed", reason: String(error) });
      });
  }, []);

  useEffect(() => {
    const wanted = debounced.trim();
    if (wanted === "") {
      setSearch({ status: "idle" });
      lastAskedRef.current = "";
      return;
    }
    if (!askable(wanted, false)) return;
    ask(wanted);
  }, [debounced, ask]);

  const type = useCallback((next: string) => {
    setQuery(next);
    if (next.trim() === "") {
      setSearch({ status: "idle" });
      lastAskedRef.current = "";
    }
  }, []);

  const now = useCallback(() => ask(queryRef.current.trim(), true), [ask]);

  const choose = useCallback((place: Place) => {
    setWeatherPlace(place);
    // Список сделал своё дело: держать его открытым незачем
    setSearch({ status: "idle" });
    lastAskedRef.current = "";
    setQuery("");
  }, []);

  return { query, search, type, now, choose };
}
