// 外观偏好先存在前端（localStorage），只作用于设置窗口。读写失败（存储不可用、内容损坏）
// 都回退到默认外观，不影响其它功能。结构参照 RambleDesk 的 appearancePreferences.ts。
import { get, writable } from "svelte/store";
import { DEFAULT_APPEARANCE, normalizeAppearance, type AppearanceSettings } from "./appearanceSettings";

export const APPEARANCE_KEY = "kotone.appearance.v1";

function readPreferences(): AppearanceSettings {
  try {
    return normalizeAppearance(JSON.parse(localStorage.getItem(APPEARANCE_KEY) ?? "null"));
  } catch {
    return { ...DEFAULT_APPEARANCE };
  }
}

const preferences = writable<AppearanceSettings>(readPreferences());
export const appearancePreferences = { subscribe: preferences.subscribe };

/** 修改并保存。保存失败时界面仍然生效，并把错误抛给调用方显示。 */
export function updateAppearance(patch: Partial<AppearanceSettings>): void {
  const next = normalizeAppearance({ ...get(preferences), ...patch });
  preferences.set(next);
  localStorage.setItem(APPEARANCE_KEY, JSON.stringify(next));
}

export function resetAppearance(): void {
  updateAppearance({ ...DEFAULT_APPEARANCE });
}

function onStorage(event: StorageEvent) {
  if (event.storageArea && event.storageArea !== localStorage) return;
  if (event.key === APPEARANCE_KEY || event.key === null) preferences.set(readPreferences());
}

/** 设置窗口挂载时调用；返回清理函数。 */
export function initializeAppearancePreferences(): () => void {
  preferences.set(readPreferences());
  window.addEventListener("storage", onStorage);
  return () => window.removeEventListener("storage", onStorage);
}
