// 插件页面的登记（ADR 038）：每个内置插件在 plugins/<id>/ui/index.ts 里给出它的图标、页面和游戏配置页里的小节，
// 这里按插件 id 列出来（一期都是内置插件，编译在一起；和桌面端 `plugins.rs` 的 registry 一一对应）。
//
// 页面照 MCP Apps：每个 `ui://kotone/<插件>/<页面>` 一个视图，同一个页面两种尺寸——聊天里的卡片（小、只读快照，
// 她那一刻看到的样子）和完整页面（右侧栏、左侧栏的插件页里，能操作、会刷新）。宿主给页面一座桥（PluginBridge）：
// 调本插件的工具（MCP Apps 的 callTool）、跳到本插件的另一个页面。
import type { Component } from "svelte";
import type { Settings } from "./ipc/contracts";
import type { PluginInfo, PluginToolResult } from "./plugins";
import lol from "$plugins/lol/ui/index";

export interface PluginPageProps {
  plugin: PluginInfo;
}
export interface PluginGameSectionProps {
  plugin: PluginInfo;
  /** 正在编辑的游戏配置 id。 */
  gameId: string;
}

/** 宿主给页面的桥（照 MCP Apps 的 App bridge）。 */
export interface PluginBridge {
  /** 调本插件的工具（只给页面用的也能调）。 */
  call(tool: string, args?: Record<string, unknown>): Promise<PluginToolResult>;
  /** 跳到本插件的另一个页面（在同一处打开，能返回）。`data` 有了就不用页面自己再读。 */
  openPage(uri: string, params?: Record<string, unknown>, data?: unknown): void;
}

/** 一个页面拿到的东西：地址、参数、数据（structuredContent；null = 页面自己去读）、桥。 */
export interface PluginViewProps {
  uri: string;
  params: Record<string, unknown>;
  data: unknown;
  bridge: PluginBridge;
}

export interface PluginView {
  /** 完整页面。 */
  page: Component<PluginViewProps>;
  /** 聊天里的卡片（小、只读快照）；没有就不出卡片。 */
  card?: Component<PluginViewProps>;
  /** 标题（右侧栏的标签、卡片头）。 */
  title: () => string;
}

export interface PluginUi {
  /** 左侧栏和插件列表里的图标。 */
  icon: Component;
  /** 插件的完整页面（左侧栏「插件」组里点开）。 */
  page: Component<PluginPageProps>;
  /** 游戏配置页里，挂在这个游戏上的插件给的小节（照 VS Code 的“按语言的设置”）。 */
  gameSection?: Component<PluginGameSectionProps>;
  /** 带游戏中心适配器的插件：现在报不报这个游戏的阶段和局内的事（游戏配置页的“游戏事件”用）。 */
  adapterOn?: (settings: Settings | null | undefined, plugin: PluginInfo) => boolean;
  /** 适配器关着时游戏配置页上的一句说明。 */
  eventsOffNote?: () => string;
  /** 按 `ui://` 地址的页面（照 MCP Apps 的 `_meta.ui.resourceUri`）。 */
  views?: Record<string, PluginView>;
  /** 聊天里工具那一行的名字（`lol.scout` → “查这局的战绩”）。 */
  toolLabels?: Record<string, () => string>;
}

export const PLUGIN_UI: Record<string, PluginUi> = { lol };

/** 有页面的插件 id（左侧栏「插件」组的顺序）。 */
export const PLUGIN_IDS: string[] = Object.keys(PLUGIN_UI);

/** `ui://kotone/<插件>/<页面>` 是哪个插件的；不是插件页面时 null。 */
export function pluginOfUri(uri: string): string | null {
  const match = /^ui:\/\/kotone\/([a-z0-9-]+)\/[a-z0-9-]+$/.exec(uri);
  return match && match[1] !== "settings" ? match[1] : null;
}

/** 某个页面地址的视图（插件和页面都登记过才有）。 */
export function viewOf(uri: string): { plugin: string; view: PluginView } | null {
  const plugin = pluginOfUri(uri);
  const view = plugin ? PLUGIN_UI[plugin]?.views?.[uri] : undefined;
  return plugin && view ? { plugin, view } : null;
}

/** 插件工具在聊天里的名字；不是插件的工具（或没写）时 null。 */
export function pluginToolLabel(tool: string): string | null {
  const plugin = tool.split(".")[0];
  return PLUGIN_UI[plugin]?.toolLabels?.[tool]?.() ?? null;
}
