// 英雄联盟插件的界面入口（ADR 038）：图标、插件页面、设置、游戏配置页里的小节、游戏中心适配器开没开，
// 以及按 `ui://` 地址的页面。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
// 页面都是插件的 MCP App（ADR 039 D5，plugins/lol/app 打成的独立 HTML）：插件页、右侧栏是 fullscreen，聊天卡片是 inline，
// 在 Kotone 和 Claude 这类宿主里是同一个 App。设置（数据开关、操作客户端）还是宿主里的 Svelte。
// 原来的两个覆盖层（选英雄、海克斯评分）改成了推送（ADR 043 D7）：选英雄、海克斯三选一都是一张卡片（App 的一页）。
import { Swords } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { sourceEnabled } from "$lib/plugins";
import { m } from "$lib/paraglide/messages.js";
import LolPage from "./LolPage.svelte";
import LolSettings from "./LolSettings.svelte";
import GameSection from "./GameSection.svelte";
import {
  PAGE_AUGMENT_OCR, PAGE_AUGMENT_PICK, PAGE_CHAMP_SELECT, PAGE_CHAMPION, PAGE_ENEMY_BUILD, PAGE_GAME, PAGE_LOL_HOME, PAGE_MATCH, PAGE_PLAYER,
  PAGE_TIER, PAGE_TODAY,
} from "./types";

const ui: PluginUi = {
  icon: Swords,
  page: LolPage,
  settings: LolSettings,
  gameSection: GameSection,
  // lol 适配器靠“读客户端数据”拿选英雄、加载、对局中这些阶段和局内的事
  adapterOn: (settings, plugin) => sourceEnabled(settings, plugin.manifest, "client"),
  eventsOffNote: () => m.plugin_lol_events_off(),
  views: {
    [PAGE_LOL_HOME]: { app: true, title: () => m.plugin_lol_page_home() },
    [PAGE_MATCH]: { app: true, title: () => m.plugin_lol_page_match() },
    [PAGE_PLAYER]: { app: true, title: () => m.plugin_lol_page_player() },
    [PAGE_GAME]: { app: true, title: () => m.plugin_lol_page_game() },
    [PAGE_CHAMPION]: { app: true, title: () => m.plugin_lol_page_champion() },
    [PAGE_TIER]: { app: true, title: () => m.plugin_lol_page_tier() },
    // 只在推送里出现的两张卡片（ADR 043 D7）
    [PAGE_CHAMP_SELECT]: { app: true, title: () => m.plugin_lol_page_champ_select() },
    [PAGE_AUGMENT_OCR]: { app: true, title: () => m.plugin_lol_page_augment_ocr() },
    [PAGE_TODAY]: { app: true, title: () => m.plugin_lol_page_today() },
    [PAGE_AUGMENT_PICK]: { app: true, title: () => m.plugin_lol_page_augment_pick() },
    [PAGE_ENEMY_BUILD]: { app: true, title: () => m.plugin_lol_page_enemy_build() },
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
    "lol.augment_ocr": () => m.plugin_lol_tool_augment_ocr(),
    "lol.import": () => m.plugin_lol_tool_import(),
  },
};

export default ui;
