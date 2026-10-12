import { createContext, useContext } from "react";

/*
 * Переход по страницам окна настроек изнутри раздела.
 *
 * Цель — та же строка, что приходит событием `settings-target`: пусто — общие
 * настройки, `mail` — страница виджета, `mail/<ключ>` и `mail/new` — страница
 * ящика (этап 53). Нужен разделам, которые ведут дальше сами: список ящиков
 * почты открывает ящик, сохранённый новый ящик переходит на свою страницу.
 * Через Rust ходить за этим незачем — страница живёт в том же окне.
 */
export const Navigate = createContext<(target: string) => void>(() => {});

export function useNavigate(): (target: string) => void {
  return useContext(Navigate);
}
