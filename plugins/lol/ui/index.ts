// 英雄联盟插件的界面入口（ADR 038）：图标、插件页面、游戏配置页里的小节、游戏中心适配器开没开，
// 以及按 `ui://` 地址的页面（照 MCP Apps：完整页面 + 聊天卡片）。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Swords } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { sourceEnabled } from "$lib/plugins";
import { m } from "$lib/paraglide/messages.js";
import LolPage from "./LolPage.svelte";
import GameSection from "./GameSection.svelte";
import MatchView from "./MatchView.svelte";
import MatchCard from "./MatchCard.svelte";
import PlayerView from "./PlayerView.svelte";
import PlayerCard from "./PlayerCard.svelte";
import GameView from "./GameView.svelte";
import { PAGE_GAME, PAGE_MATCH, PAGE_PLAYER } from "./types";

const ui: PluginUi = {
  icon: Swords,
  page: LolPage,
  gameSection: GameSection,
  // lol 适配器靠“读客户端数据”拿选英雄、加载、对局中这些阶段和局内的事
  adapterOn: (settings, plugin) => sourceEnabled(settings, plugin.manifest, "client"),
  eventsOffNote: () => m.plugin_lol_events_off(),
  views: {
    [PAGE_MATCH]: { page: MatchView, card: MatchCard, title: () => m.plugin_lol_page_match() },
    [PAGE_PLAYER]: { page: PlayerView, card: PlayerCard, title: () => m.plugin_lol_page_player() },
    [PAGE_GAME]: { page: GameView, title: () => m.plugin_lol_page_game() },
  },
  toolLabels: {
    "lol.match": () => m.plugin_lol_tool_match(),
    "lol.spell": () => m.plugin_lol_tool_spell(),
    "lol.scout": () => m.plugin_lol_tool_scout(),
    "lol.player": () => m.plugin_lol_tool_player(),
    "lol.game": () => m.plugin_lol_tool_game(),
  },
};

export default ui;
