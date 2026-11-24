// 右侧栏里打开的插件页面（ADR 038 第 2 步，照 MCP Apps：聊天卡片点开是完整页面，旁边接着和她聊）。
// 一叠页面：从聊天卡片或 `ui.open` 打开时换成新的一叠；页面里点进去（这局 → 某个人 → 某一局）往上叠，能返回。
// 只在内存里，不记（重开 Kotone 时右侧栏回到“卡片”）。
import { get, writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "../ipc";
import { pluginOfUri } from "../pluginUi";
import { openTarget } from "./navigation";
import { openRightSidebar, rightSidebarTab, setRightSidebarTab } from "./rightSidebar";

export interface OpenPage {
  uri: string;
  params: Record<string, unknown>;
  /** structuredContent（卡片点开时是她那一刻看到的样子）；null = 页面自己去读。 */
  data: unknown;
  /** 每打开一次换一个，页面据此重新挂载。 */
  key: number;
}

export const pluginPageStack = writable<OpenPage[]>([]);
let counter = 0;

function page(uri: string, params?: Record<string, unknown> | null, data?: unknown): OpenPage {
  return { uri, params: params ?? {}, data: data ?? null, key: ++counter };
}

/** 在右侧栏打开一个插件页面（换成新的一叠），切到“页面”标签。 */
export function openPluginPage(uri: string, params?: Record<string, unknown> | null, data?: unknown): void {
  pluginPageStack.set([page(uri, params, data)]);
  openRightSidebar();
  setRightSidebarTab("page");
}

/** 页面里点进下一页（能返回）。 */
export function pushPluginPage(uri: string, params?: Record<string, unknown> | null, data?: unknown): void {
  pluginPageStack.update((stack) => [...stack, page(uri, params, data)]);
}

/** 最上面一页记下自己现在的样子（参数和数据），返回到它时照这样显示。不换 key：不重新挂载。 */
export function replacePluginPage(params: Record<string, unknown>, data: unknown): void {
  pluginPageStack.update((stack) => stack.map((p, i) => (i === stack.length - 1 ? { ...p, params, data } : p)));
}

export function backPluginPage(): void {
  pluginPageStack.update((stack) => (stack.length > 1 ? stack.slice(0, -1) : stack));
}

/** 关掉右侧栏的页面，回到“卡片”。 */
export function closePluginPages(): void {
  pluginPageStack.set([]);
  if (get(rightSidebarTab) === "page") setRightSidebarTab("cards");
}

/** `ui.open` 的事件（她和外部 Agent 打开一个页面）。 */
export interface UiOpen { uri: string; params?: Record<string, unknown> | null; title?: string; plugin?: string | null }

/**
 * 处理 `ui.open`：插件的页面在聊天页的右侧栏打开（先切到聊天页）；`ui://kotone/settings/<分页>` 打开设置的那一页
 * （`ui://kotone/settings/plugins` 是旧地址：插件搬到了左侧栏的「插件」，打开那一页）。
 * 不把窗口拉到前面（玩家可能在游戏里）。认不出的地址返回 false。
 */
export function handleUiOpen(event: UiOpen): boolean {
  const uri = String(event?.uri ?? "");
  const section = /^ui:\/\/kotone\/settings\/([a-z]+)$/.exec(uri)?.[1];
  if (section) {
    openTarget(section === "plugins" ? { page: "plugins" } : { page: "advanced", tab: section });
    return true;
  }
  if (!pluginOfUri(uri)) return false;
  openTarget({ page: "assistant", section: "chat" });
  openPluginPage(uri, event.params ?? null);
  return true;
}

/** 设置窗口挂上 `kotone://ui-open`。浏览器预览里没有后端事件：给 e2e 和调试留一个 `window.kotoneUiOpen`。 */
export function listenUiOpen(): () => void {
  if (!isTauri) {
    (window as unknown as { kotoneUiOpen?: (e: UiOpen) => boolean }).kotoneUiOpen = handleUiOpen;
    return () => { delete (window as unknown as { kotoneUiOpen?: unknown }).kotoneUiOpen; };
  }
  const off = listen<UiOpen>("kotone://ui-open", (e) => { handleUiOpen(e.payload); });
  return () => { void off.then((unlisten) => unlisten()); };
}
