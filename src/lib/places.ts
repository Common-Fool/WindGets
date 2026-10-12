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

/** Короче трёх букв не ищем: находки будут случайными. */
const MIN_QUERY_LENGTH = 3;

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
    if (wanted.length < MIN_QUERY_LENGTH) return;
    if (!force && wanted === lastAskedRef.current) return;

    lastAskedRef.current = wanted;
    setSearch({ status: "busy" });

    searchPlaces(wanted)
      .then((hits) => {
        // ВАЖНО: ответ на устаревший запрос выбрасываем. Запросы уходят
        // внахлёст, и ответ на «Гра» приходит после ответа на «Грац» —
        // показанный, он подменил бы находки на чужие
        if (queryRef.current.trim() !== wanted) return;
        setSearch({ status: "done", hits });
      })
      .catch((error: unknown) => {
        if (queryRef.current.trim() !== wanted) return;
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
    if (wanted.length < MIN_QUERY_LENGTH) return;
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
