// 右侧栏里打开的插件页面（ADR 038 第 2 步，照 MCP Apps：聊天卡片点开是完整页面，旁边接着和她聊）。
// 一叠页面：从聊天卡片打开时换成新的一叠；页面里点进去（这局 → 某个人 → 某一局）往上叠，能返回。
// `ui.open` 打开时不打断玩家正在看的（同一个插件的往上叠、别的放进“卡片”等玩家点），见 stores/uiOpen.ts。
// 只在内存里，不记（重开 Kotone 时右侧栏回到“卡片”）。
import { get, writable } from "svelte/store";
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
