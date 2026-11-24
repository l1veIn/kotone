// 插件页面的登记（ADR 038）：每个内置插件在 plugins/<id>/ui/index.ts 里给出它的图标、页面和游戏配置页里的小节，
// 这里按插件 id 列出来（一期都是内置插件，编译在一起；和桌面端 `plugins.rs` 的 registry 一一对应）。
//
// 页面照 MCP Apps：每个 `ui://kotone/<插件>/<页面>` 一个视图，同一个页面两种尺寸——聊天里的卡片（小、只读快照，
// 她那一刻看到的样子）和完整页面（右侧栏、左侧栏的插件页里，能操作、会刷新）。宿主给页面一座桥（PluginBridge）：
// 调本插件的工具（MCP Apps 的 callTool）、跳到本插件的另一个页面。
import type { Component } from "svelte";
import type { Settings } from "./ipc/contracts";
import { pluginEnabled, type PluginInfo, type PluginToolResult } from "./plugins";
import lol from "$plugins/lol/ui/index";
import gameVoice from "$plugins/game-voice/ui/index";
import music from "$plugins/music/ui/index";
import interpreter from "$plugins/interpreter/ui/index";

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
  /** 记下这一页现在的样子（例如搜了别的英雄）：返回到这一页时照这样显示，不重新读。 */
  replace(params: Record<string, unknown>, data: unknown): void;
}

/** 页面参数里的这一项表示数据是页面自己查的（不是她那一刻查的快照）。调工具前要去掉。 */
export const FROM_PAGE = "_fromPage";

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

/** 左侧栏底部（设置、关于上面）那一小块拿到的东西。 */
export interface PluginSidebarFooterProps {
  plugin: PluginInfo;
  /** 左侧栏收成图标栏了：只给一个小按钮。 */
  rail: boolean;
  /** 打开这个插件的页面（从「设置 → 插件」点开的那一页）。 */
  onOpen: () => void;
}

/** 聊天输入框上面那一条拿到的东西。 */
export interface PluginChatBannerProps {
  plugin: PluginInfo;
}

/** 覆盖层上一项拿到的东西（ADR 038：覆盖层在本体里；插件只给数据，位置由宿主算好）。 */
export interface PluginOverlayProps {
  /** 插件给的数据（`OverlayItem.data`）。 */
  data: unknown;
}

export interface PluginUi {
  /** 左侧栏和插件列表里的图标。 */
  icon: Component;
  /** 插件的完整页面（从「设置 → 插件」点开；左侧栏不放插件）。 */
  page: Component<PluginPageProps>;
  /** 页面自己占满整块（自己有顶栏、底部的播放条之类），宿主不在下面留白。 */
  fullBleed?: boolean;
  /** 左侧栏底部的一小块（例如放歌的小播放条）。插件开着时才显示；要不要显示东西由它自己定。 */
  sidebarFooter?: Component<PluginSidebarFooterProps>;
  /** 聊天输入框上面的一条（例如她要放歌而你没登录网易云时的登录卡片）。插件开着时才显示。 */
  chatBanner?: Component<PluginChatBannerProps>;
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
  /** 游戏上方覆盖层的页面（按 `ui://` 地址；清单的 `overlays`）。只读、不能点，要小、要在任何画面上都看得清。 */
  overlays?: Record<string, Component<PluginOverlayProps>>;
}

export const PLUGIN_UI: Record<string, PluginUi> = { lol, "game-voice": gameVoice, music, interpreter };

/** 有页面的插件 id（「设置 → 插件」里的顺序）。 */
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

/** 插件工具在聊天里的名字；不是插件的工具（或没写）时 null。工具名的前缀不一定是插件 id（游戏语音沿用 `voice_chat.*`）。 */
export function pluginToolLabel(tool: string): string | null {
  for (const ui of Object.values(PLUGIN_UI)) {
    const label = ui.toolLabels?.[tool];
    if (label) return label();
  }
  return null;
}

/** 开着的插件里给了某一块（左侧栏底部、聊天输入框上面）的：插件和那一块的组件，按「设置 → 插件」里的顺序。 */
export function enabledSlots<K extends "sidebarFooter" | "chatBanner">(
  plugins: PluginInfo[],
  settings: Settings | null | undefined,
  slot: K,
): { plugin: PluginInfo; component: NonNullable<PluginUi[K]> }[] {
  return (plugins ?? [])
    .filter((plugin) => pluginEnabled(settings, plugin.manifest))
    .flatMap((plugin) => {
      const component = PLUGIN_UI[plugin.manifest.id]?.[slot];
      return component ? [{ plugin, component: component as NonNullable<PluginUi[K]> }] : [];
    });
}

/** 覆盖层页面（插件和地址都登记过才有）。 */
export function overlayOf(plugin: string, uri: string): Component<PluginOverlayProps> | null {
  return PLUGIN_UI[plugin]?.overlays?.[uri] ?? null;
}
