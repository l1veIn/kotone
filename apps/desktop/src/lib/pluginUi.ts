// 插件页面的登记（ADR 038）：每个内置插件在 plugins/<id>/ui/index.ts 里给出它的图标、页面和游戏配置页里的小节，
// 这里按插件 id 列出来（一期都是内置插件，编译在一起；和桌面端 `plugins.rs` 的 registry 一一对应）。
import type { Component } from "svelte";
import type { Settings } from "./ipc/contracts";
import type { PluginInfo } from "./plugins";
import lol from "$plugins/lol/ui/index";

export interface PluginPageProps {
  plugin: PluginInfo;
}
export interface PluginGameSectionProps {
  plugin: PluginInfo;
  /** 正在编辑的游戏配置 id。 */
  gameId: string;
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
}

export const PLUGIN_UI: Record<string, PluginUi> = { lol };

/** 有页面的插件 id（左侧栏「插件」组的顺序）。 */
export const PLUGIN_IDS: string[] = Object.keys(PLUGIN_UI);
