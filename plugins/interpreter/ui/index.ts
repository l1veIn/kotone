// 同传插件的界面入口（ADR 021、038）：图标、插件页面（「设置 → 插件 → 同传」）、她用 ui.open 打开的右侧栏页面。
// 工具 `interpreter.*` 只给页面用（她自己用 agent.start + voice.listen.start），聊天里不出现，所以没有 toolLabels。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Languages } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import InterpreterPage from "./InterpreterPage.svelte";
import InterpreterView from "./InterpreterView.svelte";
import { PAGE_HOME } from "./interpreter";

const ui: PluginUi = {
  icon: Languages,
  page: InterpreterPage,
  views: {
    [PAGE_HOME]: { page: InterpreterView, title: () => m.plugin_interpreter_name() },
  },
};

export default ui;
