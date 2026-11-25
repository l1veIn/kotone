// 键名：录键得到的规范写法、给人看的写法。热键冲突只在后端查（ADR 040 D6：`checkHotkey` / `getHotkeyTable`），这里不判断。

const NAMED_CODES: Record<string, string> = {
  Space: "Space",
  Tab: "Tab",
  Enter: "Enter",
  NumpadEnter: "Enter",
  Escape: "Escape",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  PrintScreen: "PrintScreen",
  Pause: "Pause",
  CapsLock: "CapsLock",
  Backquote: "Backquote",
  "`": "Backquote",
  "~": "Backquote",
};

/** 给人看的热键名：Backquote 显示成 ~。 */
export function hotkeyLabel(combo: string): string {
  return combo.split("+").map((part) => (part === "Backquote" ? "~" : part)).join(" + ");
}

/** KeyboardEvent → 与 Rust parse_hotkey 一致的规范组合名。 */
export function keyboardEventCombo(event: KeyboardEvent): string | null {
  let main: string | undefined;
  if (/^Key[A-Z]$/.test(event.code)) main = event.code.slice(3);
  else if (/^Digit[0-9]$/.test(event.code)) main = event.code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(event.code)) main = event.code;
  else main = NAMED_CODES[event.code];
  // 某些 WebView 自动化/辅助技术只填 event.key，不填物理 code；真实键盘通常
  // 两者都有，但兜底也应覆盖这一类输入源。
  if (!main) {
    if (/^[a-z0-9]$/i.test(event.key)) main = event.key.toUpperCase();
    else if (/^F([1-9]|1[0-9]|2[0-4])$/i.test(event.key)) main = event.key.toUpperCase();
    else {
      const keyCode = event.key === " " ? "Space" : event.key;
      main = NAMED_CODES[keyCode];
    }
  }
  if (!main) return null;

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(main);
  return parts.join("+");
}

/** MouseEvent 侧键 → 与 Rust parse_hotkey 一致的 Mouse4 / Mouse5 组合名。 */
export function mouseEventCombo(event: MouseEvent): string | null {
  const main = event.button === 3 ? "Mouse4" : event.button === 4 ? "Mouse5" : null;
  if (!main) return null;

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(main);
  return parts.join("+");
}

export function normalizeHotkeyCombo(combo: string): string {
  return combo
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean)
    .join("+");
}

/** 已按下组合中的主键或修饰键松开时，都应结束一次 hold。 */
export function releasesHotkey(event: KeyboardEvent, activeCombo: string): boolean {
  const activeParts = normalizeHotkeyCombo(activeCombo).split("+");
  const releasedParts = keyboardEventCombo(event)?.split("+") ?? [];
  const released = releasedParts[releasedParts.length - 1]?.toLowerCase();
  if (released && released === activeParts[activeParts.length - 1]) return true;

  const modifier =
    event.key === "Control"
      ? "ctrl"
      : event.key === "Alt"
        ? "alt"
        : event.key === "Shift"
          ? "shift"
          : null;
  return modifier !== null && activeParts.includes(modifier);
}
