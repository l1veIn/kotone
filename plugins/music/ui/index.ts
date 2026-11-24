// 放歌插件的界面入口（ADR 034、038）：图标、音乐页（左侧栏「插件 → 音乐」，自己占满整块）、她用 ui.open 打开的右侧栏页面、
// 左侧栏底部的小播放条、聊天输入框上面的登录卡片、聊天里工具那一行的名字。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Music } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import MusicPage from "./MusicPage.svelte";
import MusicView from "./MusicView.svelte";
import MusicChatBanner from "./MusicChatBanner.svelte";
import SidebarMiniPlayer from "./SidebarMiniPlayer.svelte";
import { PAGE_HOME } from "./music";

const ui: PluginUi = {
  icon: Music,
  page: MusicPage,
  fullBleed: true,
  sidebarFooter: SidebarMiniPlayer,
  chatBanner: MusicChatBanner,
  views: {
    [PAGE_HOME]: { page: MusicView, title: () => m.plugin_music_name() },
  },
  toolLabels: {
    music: () => m.plugin_music_tool_music(),
  },
};

export default ui;
