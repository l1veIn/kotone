import { get } from "svelte/store";
import { triggerLocalHotkey } from "./ipc";
import { isHotkeyCaptureActive } from "./hotkeyCapture";
import {
  keyboardEventCombo,
  normalizeHotkeyCombo,
  releasesHotkey,
} from "./hotkeyCombo";
import { settingsStore } from "./stores/ui";

export { keyboardEventCombo } from "./hotkeyCombo";

/**
 * LL 钩子按设计会先于 WebView 吞掉热键。若某个 Windows/WebView 环境仍把它
 * 送进 DOM，这里阻止页面默认行为并复用后端业务入口，保证焦点在 Kotone 内也可用。
 */
export function installLocalHotkeyBridge(): () => void {
  let activeCombo: string | null = null;
  const releaseActive = () => {
    if (!activeCombo) return;
    const combo = activeCombo;
    activeCombo = null;
    void triggerLocalHotkey(combo, false);
  };
  const handle = (event: KeyboardEvent) => {
    // 即使用户在一次 hold 尚未松开时进入录入，也必须先补齐 release；
    // 否则 capture 层会拦住 keyup，把旧会话永久留在 Listening。
    if (event.type === "keyup" && activeCombo && releasesHotkey(event, activeCombo)) {
      event.preventDefault();
      event.stopImmediatePropagation();
      releaseActive();
      return;
    }

    if (isHotkeyCaptureActive()) return;
    if (event.type !== "keydown") return;
    const configured = get(settingsStore)?.hotkey.key;
    const combo = keyboardEventCombo(event);
    if (
      !configured ||
      !combo ||
      normalizeHotkeyCombo(configured) !== normalizeHotkeyCombo(combo)
    ) {
      return;
    }

    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.repeat || activeCombo) return;
    activeCombo = combo;
    void triggerLocalHotkey(combo, true);
  };
  const handleBlur = () => releaseActive();
  const handleVisibility = () => {
    if (document.visibilityState !== "visible") releaseActive();
  };

  window.addEventListener("keydown", handle, { capture: true });
  window.addEventListener("keyup", handle, { capture: true });
  window.addEventListener("blur", handleBlur);
  document.addEventListener("visibilitychange", handleVisibility);
  return () => {
    releaseActive();
    window.removeEventListener("keydown", handle, { capture: true });
    window.removeEventListener("keyup", handle, { capture: true });
    window.removeEventListener("blur", handleBlur);
    document.removeEventListener("visibilitychange", handleVisibility);
  };
}
