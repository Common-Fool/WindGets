import { Component, type ErrorInfo, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useText } from "../lib/i18n";
import { useWindowDrag } from "../lib/drag";
import { Button } from "./Button";
import { GlassCard } from "./GlassCard";
import "./Guard.css";

/*
 * Граница отказа окна: одна на окно, в `Shell`.
 *
 * Без неё ошибка любого компонента или неприехавший кусок кода виджета
 * (ленивый `import()`) размонтировали бы всё дерево React, и окно осталось бы
 * прозрачным пятном с живым HWND: трей считает его исправным, а чинить нечем,
 * кроме перезапуска приложения. Здесь вместо пустоты — карточка с повтором и
 * дорогой к журналу. Стек человеку не показываем: он для журнала.
 */

type Props = { children: ReactNode };
type State = { failed: boolean };

export class Guard extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
    // В журнал приложения: в собранном окне консоли никто не видит
    invoke("report_crash", { message, stack: info.componentStack ?? "" }).catch(() => {});
  }

  render(): ReactNode {
    return this.state.failed ? <Crashed /> : this.props.children;
  }
}

/** Карточка отказа: окно по-прежнему тянется и открывает меню виджета. */
function Crashed() {
  const t = useText();
  useWindowDrag();

  return (
    <div className="widget">
      <GlassCard accent="neutral">
        <div className="crashed">
          <p className="crashed__text">{t("app.crashed")}</p>
          <div className="crashed__actions">
            <Button onClick={() => window.location.reload()}>{t("app.reload")}</Button>
            <Button variant="outline" onClick={() => void invoke("open_log_dir").catch(() => {})}>
              {t("app.openLog")}
            </Button>
          </div>
        </div>
      </GlassCard>
    </div>
  );
}

/**
 * Словарь не загрузился вовсе — ни свой, ни английский. Писать окну нечем, и
 * строки в коде проект не держит, поэтому остаётся одна кнопка-значок повтора.
 */
export function TextFailed() {
  useWindowDrag();

  return (
    <div className="widget">
      <GlassCard accent="neutral">
        <div className="crashed">
          <button type="button" className="crashed__retry" onClick={() => window.location.reload()}>
            <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true">
              <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>
      </GlassCard>
    </div>
  );
}
