// 英雄联盟插件的界面入口（ADR 038）：图标、插件页面、游戏配置页里的小节，以及游戏中心适配器开没开。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Swords } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { sourceEnabled } from "$lib/plugins";
import { m } from "$lib/paraglide/messages.js";
import LolPage from "./LolPage.svelte";
import GameSection from "./GameSection.svelte";

const ui: PluginUi = {
  icon: Swords,
  page: LolPage,
  gameSection: GameSection,
  // lol 适配器靠“读客户端数据”拿选英雄、加载、对局中这些阶段和局内的事
  adapterOn: (settings, plugin) => sourceEnabled(settings, plugin.manifest, "client"),
  eventsOffNote: () => m.plugin_lol_events_off(),
};

export default ui;
