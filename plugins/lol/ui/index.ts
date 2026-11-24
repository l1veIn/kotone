// 英雄联盟插件的界面入口（ADR 038）：图标、插件页面、游戏配置页里的小节、游戏中心适配器开没开，
// 以及按 `ui://` 地址的页面（照 MCP Apps：完整页面 + 聊天卡片）和游戏上方的覆盖层。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
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
import ChampionView from "./ChampionView.svelte";
import ChampionCard from "./ChampionCard.svelte";
import ChampSelectOverlay from "./ChampSelectOverlay.svelte";
import AugmentOverlay from "./AugmentOverlay.svelte";
import {
  OVERLAY_AUGMENTS, OVERLAY_CHAMP_SELECT, PAGE_CHAMPION, PAGE_GAME, PAGE_MATCH, PAGE_PLAYER, PAGE_TIER,
} from "./types";

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
    [PAGE_CHAMPION]: { page: ChampionView, card: ChampionCard, title: () => m.plugin_lol_page_champion() },
    // 强度榜是 MCP App（ADR 039 D5）：plugins/lol/app 打成的独立 HTML，在 Kotone 和 Claude 这类宿主里是同一个页面
    [PAGE_TIER]: { app: true, title: () => m.plugin_lol_page_tier() },
  },
  toolLabels: {
    "lol.match": () => m.plugin_lol_tool_match(),
    "lol.spell": () => m.plugin_lol_tool_spell(),
    "lol.scout": () => m.plugin_lol_tool_scout(),
    "lol.player": () => m.plugin_lol_tool_player(),
    "lol.game": () => m.plugin_lol_tool_game(),
    "lol.champion": () => m.plugin_lol_tool_champion(),
    "lol.tier": () => m.plugin_lol_tool_tier(),
    "lol.client": () => m.plugin_lol_tool_client(),
    "lol.champions": () => m.plugin_lol_tool_champions(),
  },
  // 游戏上方的覆盖层（照 aramhex）：选英雄时的胜率、海斗三选一时的海克斯评分
  overlays: {
    [OVERLAY_CHAMP_SELECT]: ChampSelectOverlay,
    [OVERLAY_AUGMENTS]: AugmentOverlay,
  },
};

export default ui;
