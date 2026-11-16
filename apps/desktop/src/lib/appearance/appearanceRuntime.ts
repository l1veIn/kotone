// 把外观偏好应用到当前窗口的 <html>。只由设置窗口调用：悬浮窗与弹幕窗不调用，保持原样。
// 参照 RambleDesk appearanceRuntime.ts；缩放改用 CSS zoom（不需要额外的窗口权限），
// 也不注册 Ctrl +/- 快捷键（会和热键录制冲突）。
import { writable } from "svelte/store";
import { initializeAppearancePreferences, appearancePreferences } from "./appearancePreferences";
import { isDefaultLook, resolveFontFamily, type AppearanceSettings } from "./appearanceSettings";

/** 当前实际的明暗（提示组件等需要知道）。 */
export const resolvedTheme = writable<"dark" | "light">("dark");

export function resolveTheme(mode: AppearanceSettings["mode"], prefersDark: boolean): "dark" | "light" {
  return mode === "system" ? (prefersDark ? "dark" : "light") : mode;
}

export function applyAppearance(root: HTMLElement, settings: AppearanceSettings, prefersDark: boolean): void {
  const theme = resolveTheme(settings.mode, prefersDark);
  root.dataset.appearance = "enabled";
  root.dataset.palette = settings.palette;
  root.dataset.theme = theme;
  resolvedTheme.set(theme);
  // 非默认外观时，旧页面的固定颜色（text-white、bg-kotone-* …）改用主题变量（见 app.css）。
  if (isDefaultLook(settings.palette, theme)) delete root.dataset.bridge;
  else root.dataset.bridge = "on";
  root.dataset.uiFont = settings.uiFont;
  root.style.colorScheme = theme;
  root.style.setProperty("--appearance-ui-font", resolveFontFamily(settings));
  document.body.style.setProperty("zoom", settings.zoom === 100 ? "" : String(settings.zoom / 100));
}

/** 设置窗口挂载时调用；返回清理函数（恢复成未设置外观的状态）。 */
export function initializeAppearance(root: HTMLElement = document.documentElement): () => void {
  const releasePreferences = initializeAppearancePreferences();
  const media = window.matchMedia?.("(prefers-color-scheme: dark)");
  let current: AppearanceSettings | null = null;
  const apply = () => { if (current) applyAppearance(root, current, media?.matches ?? true); };
  const unsubscribe = appearancePreferences.subscribe((settings) => { current = settings; apply(); });
  media?.addEventListener("change", apply);
  return () => {
    unsubscribe();
    media?.removeEventListener("change", apply);
    releasePreferences();
    for (const key of ["appearance", "palette", "theme", "bridge", "uiFont"]) delete root.dataset[key];
    root.style.removeProperty("color-scheme");
    root.style.removeProperty("--appearance-ui-font");
    document.body.style.removeProperty("zoom");
  };
}
