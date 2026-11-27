// CS2 插件的界面入口（ADR 036 D9、ADR 047 第 3 期）：图标、插件的 App（左侧栏「插件 → CS2」）、她用 ui.open 打开的页面、
// 两种卡片（这一局 / 比赛结束；下包倒数）。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Crosshair } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import BombCard from "./BombCard.svelte";
import Cs2Page from "./Cs2Page.svelte";
import Cs2Settings from "./Cs2Settings.svelte";
import Cs2View from "./Cs2View.svelte";
import MatchCard from "./MatchCard.svelte";
import { PAGE_CS2_BOMB, PAGE_CS2_HOME, PAGE_CS2_MATCH } from "./cs2";

const ui: PluginUi = {
  icon: Crosshair,
  page: Cs2Page,
  settings: Cs2Settings,
  views: {
    [PAGE_CS2_HOME]: { page: Cs2View, title: () => m.plugin_cs2_name() },
    [PAGE_CS2_MATCH]: { card: MatchCard, page: MatchCard, title: () => m.plugin_cs2_section_match() },
    [PAGE_CS2_BOMB]: { card: BombCard, page: BombCard, title: () => m.plugin_cs2_push_bomb_title() },
  },
  toolLabels: {
    "cs2.state": () => m.plugin_cs2_tool_state(),
    "cs2.rounds": () => m.plugin_cs2_tool_rounds(),
  },
};

export default ui;
