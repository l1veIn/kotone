import { get } from "svelte/store";
import { triggerLocalHotkey } from "./ipc";
import { isHotkeyCaptureActive } from "./hotkeyCapture";
import { settingsStore } from "./stores/ui";

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
};

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

function normalized(combo: string): string {
  return combo
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean)
    .join("+");
}

/**
 * LL 钩子按设计会先于 WebView 吞掉热键。若某个 Windows/WebView 环境仍把它
 * 送进 DOM，这里阻止页面默认行为并复用后端业务入口，保证焦点在 Kotone 内也可用。
 */
export function installLocalHotkeyBridge(): () => void {
  const handle = (event: KeyboardEvent) => {
    if (isHotkeyCaptureActive()) return;
    const configured = get(settingsStore)?.hotkey.key;
    const combo = keyboardEventCombo(event);
    if (!configured || !combo || normalized(configured) !== normalized(combo)) return;

    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.repeat) return;
    void triggerLocalHotkey(combo, event.type === "keydown");
  };

  window.addEventListener("keydown", handle, { capture: true });
  window.addEventListener("keyup", handle, { capture: true });
  return () => {
    window.removeEventListener("keydown", handle, { capture: true });
    window.removeEventListener("keyup", handle, { capture: true });
  };
}
