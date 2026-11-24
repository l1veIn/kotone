// 聊天页右侧栏（ADR 031，照 DeepSeek Harness 桌面版的右侧栏：标签页、拖动调宽）：
// - 开着还是收起：宽窗口时记住玩家的选择；窄窗口时默认收起，打开是临时盖在聊天上的，不记。
// - 宽度：拖动调宽，记住；双击分隔线恢复默认。
// - 标签：“卡片”（网页卡片 + 后台在做的事）/ “网页”（浏览器占满）/ “页面”（插件页面，ADR 038）；记住前两个。
// - 网页全屏：铺满整个 Kotone 窗口，不记。
// 都存本机 localStorage，读写失败就按默认（隐私窗口、清过数据时也能正常显示）。
import { derived, get, writable } from "svelte/store";

const OPEN_KEY = "kotone.chat.rightSidebar";
const WIDTH_KEY = "kotone.chat.rightSidebarWidth";
const TAB_KEY = "kotone.chat.rightSidebarTab";

function read(key: string): string | null {
  try { return localStorage.getItem(key); } catch { return null; }
}
function write(key: string, value: string): void {
  try { localStorage.setItem(key, value); } catch { /* 存不了就只在这次有效 */ }
}

// ---- 开 / 关 ----

/** 宽窗口时右侧栏开不开；null = 玩家没选过（默认开）。 */
export const rightSidebarPreference = writable<boolean | null>(
  (() => { const v = read(OPEN_KEY); return v === "open" ? true : v === "closed" ? false : null; })(),
);

export function setRightSidebarPreference(open: boolean): void {
  rightSidebarPreference.set(open);
  write(OPEN_KEY, open ? "open" : "closed");
}

/** 窄窗口时临时打开的右侧栏（盖在聊天上，不记）。 */
export const rightSidebarOverlay = writable(false);

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

// ---- 宽度（拖动调宽） ----

/** 默认宽度（双击分隔线恢复）、最窄；最宽另按聊天页宽度算（聊天区至少留 CHAT_KEEP）。 */
export const SIDEBAR_DEFAULT_WIDTH = 420;
export const SIDEBAR_MIN_WIDTH = 300;
export const SIDEBAR_MAX_WIDTH = 1400;
/** 拖宽右侧栏时聊天区至少留这么宽。 */
export const CHAT_KEEP = 400;

/** 这么宽的聊天页上右侧栏最宽能多宽。 */
export function maxSidebarWidth(chatWidth: number): number {
  if (chatWidth <= 0) return SIDEBAR_MAX_WIDTH;
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, chatWidth - CHAT_KEEP));
}

export function clampSidebarWidth(width: number, chatWidth: number): number {
  return Math.round(Math.min(maxSidebarWidth(chatWidth), Math.max(SIDEBAR_MIN_WIDTH, width)));
}

/** 玩家拖出来的宽度（像素）；没拖过是默认。 */
export const rightSidebarWidth = writable<number>(
  (() => { const v = Number(read(WIDTH_KEY)); return Number.isFinite(v) && v >= SIDEBAR_MIN_WIDTH ? v : SIDEBAR_DEFAULT_WIDTH; })(),
);

/** 拖完（或双击恢复）时记住宽度。 */
export function saveRightSidebarWidth(width: number): void {
  rightSidebarWidth.set(width);
  write(WIDTH_KEY, String(Math.round(width)));
}

// ---- 标签页 ----

/** “页面”：右侧栏里打开的插件页面（ADR 038，stores/pluginPages.ts）；不记，重开时回到“卡片”。 */
export type SidebarTab = "cards" | "web" | "page";

/** 选中的标签（记住）。“网页”只在打开过网页时生效，否则显示“卡片”。 */
export const rightSidebarTab = writable<SidebarTab>(read(TAB_KEY) === "web" ? "web" : "cards");

export function setRightSidebarTab(tab: SidebarTab): void {
  rightSidebarTab.set(tab);
  write(TAB_KEY, tab);
}

/** 网页卡片上的“展开” / 要玩家登录：打开右侧栏并切到“网页”。 */
export function showBrowserTab(): void {
  openRightSidebar();
  setRightSidebarTab("web");
}

// ---- 网页全屏 ----

/** 网页铺满整个 Kotone 窗口（不记）。 */
export const browserFullscreen = writable(false);
