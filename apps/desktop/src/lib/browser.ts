// 内置浏览器（ADR 031）：聊天页右侧栏里那块网页。网页本身由后端摆在窗口里（盖在界面上），
// 前端只负责：告诉后端那块区域在哪、显示它的状态（网站、在放的歌、要不要玩家来登录）、几个按钮。
// 后端：apps/desktop/src-tauri/src/browser/。浏览器预览（dev:web）用 ipc/browserMock 里的内存状态。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readable, type Readable } from "svelte/store";
import { isTauri } from "./ipc";
import * as browserMock from "./ipc/browserMock";

export interface BrowserMedia { playing: boolean; title: string | null; artist: string | null }
export interface BrowserState {
  /** 打开过网页（页面还在，收起时也在）。 */
  open: boolean;
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
}
export type BrowserAction = "reload" | "back" | "close" | "play" | "pause" | "next";
/** 浏览器区域在窗口里的位置（逻辑像素，相对窗口内容区左上角）。 */
export interface BrowserBounds { x: number; y: number; width: number; height: number; visible: boolean }

export const BROWSER_EVENT = "kotone://browser";
export const CLOSED_BROWSER: BrowserState = { open: false, url: null, title: null, site: null, loading: false, handover: null, media: null, busy: false, error: null };

/** 后端给的状态补齐缺省字段（旧后端、模拟数据少字段也能显示）。 */
export function normalizeBrowserState(value: Partial<BrowserState> | null | undefined): BrowserState {
  if (!value || typeof value !== "object") return { ...CLOSED_BROWSER };
  return {
    open: !!value.open,
    url: value.url ?? null,
    title: value.title ?? null,
    site: value.site ?? null,
    loading: !!value.loading,
    handover: value.handover && typeof value.handover.reason === "string" ? { reason: value.handover.reason } : null,
    media: value.media ? { playing: !!value.media.playing, title: value.media.title ?? null, artist: value.media.artist ?? null } : null,
    busy: !!value.busy,
    error: typeof value.error === "string" && value.error.trim() ? value.error : null,
  };
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
  if (!media.title) return media.playing ? "正在放歌" : "暂停了";
  const song = `《${media.title}》${media.artist ? `- ${media.artist}` : ""}`;
  return media.playing ? `正在放${song}` : `暂停了：${song}`;
}

/** 网站的叫法：常见音乐站给中文名，其余显示域名。 */
const SITE_NAMES: Record<string, string> = {
  "music.163.com": "网易云音乐", "y.qq.com": "QQ 音乐", "www.kugou.com": "酷狗音乐", "kugou.com": "酷狗音乐",
  "www.kuwo.cn": "酷我音乐", "kuwo.cn": "酷我音乐",
};
export function siteName(state: Pick<BrowserState, "site" | "url">): string {
  const site = state.site ?? (state.url ? safeHost(state.url) : null);
  if (!site) return "网页";
  return SITE_NAMES[site] ?? site.replace(/^www\./, "");
}
function safeHost(url: string): string | null {
  try { return new URL(url).host || null; } catch { return null; }
}
