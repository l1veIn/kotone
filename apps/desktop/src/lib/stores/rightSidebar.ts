// 聊天页右侧栏（ADR 031）开着还是收起：宽窗口时记住玩家的选择（存本机 localStorage，读写失败就按默认）；
// 窄窗口时默认收起，打开是临时盖在聊天上的，不记。浏览器区域展开与否不记（每次打开软件从卡片开始）。
import { derived, get, writable } from "svelte/store";

const KEY = "kotone.chat.rightSidebar";

function readPreference(): boolean | null {
  try {
    const value = localStorage.getItem(KEY);
    return value === "open" ? true : value === "closed" ? false : null;
  } catch {
    return null;
  }
}

/** 宽窗口时右侧栏开不开；null = 玩家没选过（默认开）。 */
export const rightSidebarPreference = writable<boolean | null>(readPreference());

export function setRightSidebarPreference(open: boolean): void {
  rightSidebarPreference.set(open);
  try { localStorage.setItem(KEY, open ? "open" : "closed"); } catch { /* 存不了就只在这次有效 */ }
}

/** 窄窗口时临时打开的右侧栏（盖在聊天上，不记）。 */
export const rightSidebarOverlay = writable(false);

/** 浏览器区域展开（右侧栏变宽、网页出现）。 */
export const browserExpanded = writable(false);

/** 聊天区够宽才把右侧栏并排放；更窄时右侧栏盖在聊天上、默认收起。 */
export const RIGHT_SIDEBAR_MIN_CHAT = 900;

/** 聊天页现在是不是窄（由聊天页量出来）。 */
export const chatNarrow = writable(false);

/** 右侧栏现在开着吗：宽窗口按玩家的选择（默认开），窄窗口按临时开关（默认关）。 */
export const rightSidebarOpen = derived(
  [rightSidebarPreference, rightSidebarOverlay, chatNarrow],
  ([preference, overlay, narrow]) => (narrow ? overlay : (preference ?? true)),
);

/** 标题栏的按钮：宽窗口改玩家的选择（记住），窄窗口只开关临时的那层。 */
export function toggleRightSidebar(): void {
  if (get(chatNarrow)) rightSidebarOverlay.update((open) => !open);
  else setRightSidebarPreference(!(get(rightSidebarPreference) ?? true));
}

/** 打开右侧栏（例如要玩家来登录时）。 */
export function openRightSidebar(): void {
  if (get(chatNarrow)) rightSidebarOverlay.set(true);
  else if (get(rightSidebarPreference) === false) setRightSidebarPreference(true);
}
