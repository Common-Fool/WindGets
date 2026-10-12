/*
 * Копирование в буфер обмена: одно на все виджеты.
 *
 * Переехало из `password.ts`, когда копировать понадобилось и заметке. Второй
 * копии тут быть не должно — запасной путь ниже нужен ровно в тех редких
 * случаях, когда современный доступ к буферу закрыт, и разойтись двум таким
 * развилкам было бы вопросом времени.
 */

/**
 * Кладёт текст в буфер обмена.
 *
 * Запасной путь через `execCommand` устарел, но в WebView2 современный доступ
 * к буферу иногда закрыт, а «скопировать» без копирования — худшее, чем может
 * кончиться нажатие.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return copyThroughSelection(text);
  }
}

function copyThroughSelection(text: string): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  // Вне экрана, но в документе: невидимое поле выделить нельзя
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.appendChild(field);
  field.select();

  try {
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    field.remove();
  }
}
