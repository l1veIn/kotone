// 内置浏览器（ADR 031）：聊天页右侧栏里那块网页。网页本身由后端摆在窗口里（盖在界面上），
// 前端只负责：告诉后端那块区域在哪、显示它的状态（网站、网页里在放什么、要不要玩家来登录）、几个按钮。
// 后端：apps/desktop/src-tauri/src/browser/。浏览器预览（dev:web）用 ipc/browserMock 里的内存状态。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readable, type Readable } from "svelte/store";
import { isTauri } from "./ipc";
import * as browserMock from "./ipc/browserMock";
import { m } from "./paraglide/messages.js";

export interface BrowserMedia { playing: boolean; title: string | null; artist: string | null }
/** 一个标签页（照 Codex / DSH 的内置浏览器：多标签、地址栏）。 */
export interface BrowserTab {
  /** 稳定 id。 */
  id: string;
  url: string | null;
  title: string | null;
  site: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  /** 这个标签页在出声。 */
  audible: boolean;
  /** 她（后台任务）在用的标签页。 */
  agent: boolean;
  /** 页面里有元素要全屏（例如视频的全屏按钮）。 */
  fullscreenElement: boolean;
}
export interface BrowserState {
  /** 打开过网页（页面还在，收起时也在）。 */
  open: boolean;
  /** 以下 url / title / site / loading 都是当前标签页的。 */
  url: string | null;
  title: string | null;
  /** 域名，如 "music.163.com"。 */
  site: string | null;
  loading: boolean;
  /** 需要玩家来（登录、验证码、付款）。 */
  handover: { reason: string } | null;
  /** 页面里在放的歌。 */
  media: BrowserMedia | null;
  /** 她正在操作这个页面。 */
  busy: boolean;
  /** 浏览器没能打开或坏了的原因（例如“浏览器没能打开：……”）；正常时为 null。 */
  error: string | null;
  /** 标签页（旧后端没有时按上面的字段补一个）。 */
  tabs: BrowserTab[];
  activeTab: string | null;
  /** 一句提示（例如“下载被拒绝了”）；玩家可以关掉。 */
  notice: string | null;
}
/** 网页上的按钮（放歌不在这里：音乐页，ADR 034）。 */
export type BrowserAction = "reload" | "back" | "forward" | "stop" | "close";
/** 浏览器区域在窗口里的位置（逻辑像素，相对窗口内容区左上角）。 */
export interface BrowserBounds { x: number; y: number; width: number; height: number; visible: boolean }
/** 一个扩展（已解压的 Chrome / Edge 扩展文件夹；界面上叫“扩展”，不叫插件）。 */
export interface BrowserExtension { id: string; name: string; enabled: boolean; path: string }

export const BROWSER_EVENT = "kotone://browser";
export const CLOSED_BROWSER: BrowserState = {
  open: false, url: null, title: null, site: null, loading: false, handover: null, media: null, busy: false, error: null,
  tabs: [], activeTab: null, notice: null,
};

const text = (value: unknown): string | null => (typeof value === "string" && value.trim() ? value : null);

function normalizeTab(value: Partial<BrowserTab> | null | undefined, index: number): BrowserTab | null {
  if (!value || typeof value !== "object") return null;
  return {
    id: typeof value.id === "string" && value.id ? value.id : `tab-${index}`,
    url: value.url ?? null,
    title: value.title ?? null,
    site: value.site ?? null,
    loading: !!value.loading,
    canGoBack: !!value.canGoBack,
    canGoForward: !!value.canGoForward,
    audible: !!value.audible,
    agent: !!value.agent,
    fullscreenElement: !!value.fullscreenElement,
  };
}

/** 后端给的状态补齐缺省字段（旧后端、模拟数据少字段也能显示）。 */
export function normalizeBrowserState(value: Partial<BrowserState> | null | undefined): BrowserState {
  if (!value || typeof value !== "object") return { ...CLOSED_BROWSER };
  const open = !!value.open;
  const media = value.media ? { playing: !!value.media.playing, title: value.media.title ?? null, artist: value.media.artist ?? null } : null;
  let tabs = Array.isArray(value.tabs)
    ? value.tabs.map((t, i) => normalizeTab(t, i)).filter((t): t is BrowserTab => !!t)
    : [];
  // 旧后端只有一个页面：补一个标签页，界面照样能用
  if (!tabs.length && open) {
    tabs = [{
      id: "main", url: value.url ?? null, title: value.title ?? null, site: value.site ?? null, loading: !!value.loading,
      canGoBack: false, canGoForward: false, audible: !!media?.playing, agent: !!value.busy, fullscreenElement: false,
    }];
  }
  const activeTab = typeof value.activeTab === "string" && tabs.some((t) => t.id === value.activeTab)
    ? value.activeTab : (tabs[0]?.id ?? null);
  return {
    open,
    url: value.url ?? null,
    title: value.title ?? null,
    site: value.site ?? null,
    loading: !!value.loading,
    handover: value.handover && typeof value.handover.reason === "string" ? { reason: value.handover.reason } : null,
    media,
    busy: !!value.busy,
    error: text(value.error),
    tabs,
    activeTab,
    notice: text(value.notice),
  };
}

/** 当前标签页（没有时为 null）。 */
export function activeTabOf(state: BrowserState): BrowserTab | null {
  return state.tabs.find((t) => t.id === state.activeTab) ?? null;
}

/** 标签页上显示的名字：标题，其次网站名，新标签页写“新标签页”。 */
export function tabLabel(tab: BrowserTab): string {
  if (tab.title?.trim()) return tab.title;
  if (tab.site || tab.url) return siteName(tab);
  return m.misc_browser_new_tab();
}

export async function browserState(): Promise<BrowserState> {
  if (!isTauri) return normalizeBrowserState(browserMock.browserState());
  return normalizeBrowserState(await invoke<BrowserState | null>("browser_state"));
}
export async function browserSetBounds(bounds: BrowserBounds): Promise<void> {
  if (!isTauri) { browserMock.browserSetBounds(bounds); return; }
  await invoke("browser_set_bounds", { ...bounds });
}
export async function browserControl(action: BrowserAction): Promise<void> {
  if (!isTauri) { browserMock.browserControl({ action }); return; }
  await invoke("browser_control", { action });
}
export async function browserHandoverDone(): Promise<void> {
  if (!isTauri) { browserMock.browserHandoverDone(); return; }
  await invoke("browser_handover_done");
}
export async function browserClearData(): Promise<void> {
  if (!isTauri) { browserMock.browserClearData(); return; }
  await invoke("browser_clear_data");
}

/** 地址栏回车：像网址就打开，否则当搜索词（后端决定）。 */
export async function browserNavigate(url: string): Promise<void> {
  if (!isTauri) { browserMock.browserNavigate({ url }); return; }
  await invoke("browser_navigate", { url });
}
export async function browserTabNew(url?: string): Promise<void> {
  if (!isTauri) { browserMock.browserTabNew({ url }); return; }
  await invoke("browser_tab_new", url ? { url } : {});
}
export async function browserTabClose(id: string): Promise<void> {
  if (!isTauri) { browserMock.browserTabClose({ id }); return; }
  await invoke("browser_tab_close", { id });
}
export async function browserTabSelect(id: string): Promise<void> {
  if (!isTauri) { browserMock.browserTabSelect({ id }); return; }
  await invoke("browser_tab_select", { id });
}

// ---- 扩展（设置 → 浏览器） ----

function normalizeExtension(value: Partial<BrowserExtension> | null | undefined): BrowserExtension | null {
  if (!value || typeof value !== "object" || typeof value.id !== "string") return null;
  return { id: value.id, name: text(value.name) ?? value.id, enabled: value.enabled !== false, path: typeof value.path === "string" ? value.path : "" };
}
export async function browserExtensions(): Promise<BrowserExtension[]> {
  const list = !isTauri ? browserMock.browserExtensions() : await invoke<Partial<BrowserExtension>[] | null>("browser_extensions");
  return (list ?? []).map(normalizeExtension).filter((e): e is BrowserExtension => !!e);
}
export async function browserExtensionAdd(path: string): Promise<void> {
  if (!isTauri) { browserMock.browserExtensionAdd({ path }); return; }
  await invoke("browser_extension_add", { path });
}
export async function browserExtensionRemove(id: string): Promise<void> {
  if (!isTauri) { browserMock.browserExtensionRemove({ id }); return; }
  await invoke("browser_extension_remove", { id });
}
export async function browserExtensionToggle(id: string, enabled: boolean): Promise<void> {
  if (!isTauri) { browserMock.browserExtensionToggle({ id, enabled }); return; }
  await invoke("browser_extension_toggle", { id, enabled });
}

export async function subscribeBrowser(callback: (state: BrowserState) => void): Promise<UnlistenFn> {
  if (!isTauri) return browserMock.subscribeBrowserPreview((state) => callback(normalizeBrowserState(state)));
  return listen<BrowserState>(BROWSER_EVENT, (e) => callback(normalizeBrowserState(e.payload)));
}

/** 浏览器状态：先读一次，之后跟着事件更新（有人订阅时才连上）。 */
export const browserStore: Readable<BrowserState> = readable<BrowserState>({ ...CLOSED_BROWSER }, (set) => {
  let alive = true;
  let unlisten: UnlistenFn | undefined;
  let latestEvent = 0;
  void subscribeBrowser((state) => { latestEvent += 1; set(state); })
    .then((un) => { if (alive) unlisten = un; else un(); })
    .catch(() => {});
  const seen = latestEvent;
  void browserState()
    .then((state) => { if (alive && latestEvent === seen) set(state); })
    .catch(() => {});
  return () => { alive = false; unlisten?.(); };
});

/** “正在放《晴天》- 周杰伦”；没有歌名时只说在放。 */
export function mediaLine(media: BrowserMedia): string {
  if (!media.title) return media.playing ? m.misc_browser_media_playing() : m.misc_browser_media_paused();
  const song = media.artist
    ? m.misc_browser_media_song_artist({ title: media.title, artist: media.artist })
    : m.misc_browser_media_song({ title: media.title });
  return media.playing ? m.misc_browser_media_playing_song({ song }) : m.misc_browser_media_paused_song({ song });
}

/** 网站的叫法：常见音乐站给中文名，其余显示域名。 */
const SITE_NAMES: Record<string, string> = {
  "music.163.com": m.misc_browser_site_netease_music(), "y.qq.com": m.misc_browser_site_qq_music(),
  "www.kugou.com": m.misc_browser_site_kugou_music(), "kugou.com": m.misc_browser_site_kugou_music(),
  "www.kuwo.cn": m.misc_browser_site_kuwo_music(), "kuwo.cn": m.misc_browser_site_kuwo_music(),
};
export function siteName(state: Pick<BrowserState, "site" | "url">): string {
  const site = state.site ?? (state.url ? safeHost(state.url) : null);
  if (!site) return m.misc_browser_site_web_page();
  return SITE_NAMES[site] ?? site.replace(/^www\./, "");
}
function safeHost(url: string): string | null {
  try { return new URL(url).host || null; } catch { return null; }
}
