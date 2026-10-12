import { useEffect, useLayoutEffect, useRef, useState } from "react";

/*
 * Подсказки окна настроек: пояснение к настройке всплывает при наведении, а не
 * стоит под ней абзацем.
 *
 * Решение пользователя (28 сентября): разделы превращались в полотна текста,
 * хотя большинство настроек понятны по одному имени. Пояснение осталось, но
 * ушло в атрибут `data-hint` на самой настройке, а рисует его одна на окно
 * всплывашка отсюда. Абзацем под настройкой осталось только то, без чего её не
 * настроить: пароль приложения у почты, Premium у Спотифая, шифрование кодов
 * 2FA, — и списки в «Виджетах», цветах и «Данных».
 *
 * Своя, а не системная `title`: та рисуется светлой плашкой Windows поверх
 * тёмного окна и ждёт секунду. Родное `title` осталось у значков и галочек
 * таблицы — там это имя, а не пояснение.
 */

/** Сколько мышь стоит на настройке, прежде чем подсказка всплывёт: пролёт мимо её не зовёт. */
const DELAY = 400;

/** Зазор между настройкой и подсказкой. */
const GAP = 6;

/** Ближе к краю окна подсказка не встаёт. */
const EDGE = 8;

type Shown = { text: string; anchor: DOMRect };

export function Hints() {
  const [shown, setShown] = useState<Shown | null>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);
  const bubble = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let current: HTMLElement | null = null;
    let timer = 0;

    const hide = () => {
      window.clearTimeout(timer);
      current = null;
      setShown(null);
    };

    const over = (event: PointerEvent) => {
      const target = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-hint]") : null;
      if (target === current) return;

      hide();
      // С нажатой кнопкой тянут ползунок или окно — не до подсказок
      if (target === null || event.buttons !== 0) return;

      current = target;
      timer = window.setTimeout(() => {
        const text = target.dataset.hint;
        if (text === undefined || text === "" || !target.isConnected) return;
        // Место считается заново: подсказка сперва встаёт в угол невидимой и
        // меряется там — у края окна её ширину урезало бы
        setPlace(null);
        setShown({ text, anchor: target.getBoundingClientRect() });
      }, DELAY);
    };

    // Мышь ушла из окна: переходить ей больше не к кому
    const out = (event: PointerEvent) => {
      if (event.relatedTarget === null) hide();
    };

    document.addEventListener("pointerover", over);
    document.addEventListener("pointerout", out);
    // Нажатие, клавиша и прокрутка уводят внимание или саму настройку
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("keydown", hide, true);
    document.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);

    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("pointerover", over);
      document.removeEventListener("pointerout", out);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("keydown", hide, true);
      document.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, []);

  // Под настройкой, а не влезает — над ней; и не за край окна
  useLayoutEffect(() => {
    if (shown === null || place !== null || bubble.current === null) return;

    const { width, height } = bubble.current.getBoundingClientRect();
    const { anchor } = shown;
    const left = Math.max(EDGE, Math.min(anchor.left, window.innerWidth - width - EDGE));
    const below = anchor.bottom + GAP;
    const top =
      below + height <= window.innerHeight - EDGE ? below : Math.max(EDGE, anchor.top - GAP - height);

    setPlace({ left, top });
  }, [shown, place]);

  if (shown === null) return null;

  return (
    <div
      ref={bubble}
      className="hint"
      role="tooltip"
      style={place === null ? { left: 0, top: 0, visibility: "hidden" } : place}
    >
      {shown.text}
    </div>
  );
}
