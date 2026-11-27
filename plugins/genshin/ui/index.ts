// 原神插件的界面入口（ADR 047 第 3 期）：图标、插件的 App（左侧栏「插件 → 原神」）、她用 ui.open 打开的右侧栏页面、
// 推送“BetterGI 的运行”的卡片（聊天里的卡片和点开以后右侧栏里是同一个组件）。宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Sparkles } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import GenshinPage from "./GenshinPage.svelte";
import GenshinSettings from "./GenshinSettings.svelte";
import GenshinView from "./GenshinView.svelte";
import RunCard from "./RunCard.svelte";
import { PAGE_HOME, PAGE_RUN } from "./genshin";

const ui: PluginUi = {
  icon: Sparkles,
  page: GenshinPage,
  settings: GenshinSettings,
  views: {
    [PAGE_HOME]: { page: GenshinView, title: () => m.plugin_genshin_name() },
    [PAGE_RUN]: { card: RunCard, page: RunCard, title: () => m.plugin_genshin_push_runs_title() },
  },
  toolLabels: {
    "genshin.list": () => m.plugin_genshin_tool_list(),
    "genshin.status": () => m.plugin_genshin_tool_status(),
    "genshin.run": () => m.plugin_genshin_tool_run(),
    "genshin.stop": () => m.plugin_genshin_tool_stop(),
  },
};

export default ui;
