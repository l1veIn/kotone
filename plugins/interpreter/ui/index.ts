// 同传插件的界面入口（ADR 021、038）：图标、插件的 App（左侧栏「插件 → 同传」）、她用 ui.open 打开的右侧栏页面。
// 页面的工具 `interpreter.status|start|stop` 只给页面用（她自己用 agent.start + voice.listen.start）；后台每翻好一句调的
// `interpreter.show` 在后台那段对话里，叫“显示同传的一句”。推送“这局的同传”的卡片（ADR 043 D11）：聊天里的卡片和点开以后
// 右侧栏里是同一个组件。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Languages } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import InterpreterPage from "./InterpreterPage.svelte";
import InterpreterSettings from "./InterpreterSettings.svelte";
import InterpreterView from "./InterpreterView.svelte";
import LinesCard from "./LinesCard.svelte";
import { PAGE_HOME, PAGE_LINES } from "./interpreter";

const ui: PluginUi = {
  icon: Languages,
  page: InterpreterPage,
  settings: InterpreterSettings,
  views: {
    [PAGE_HOME]: { page: InterpreterView, title: () => m.plugin_interpreter_name() },
    [PAGE_LINES]: { card: LinesCard, page: LinesCard, title: () => m.plugin_interpreter_push_lines_title() },
  },
  toolLabels: {
    "interpreter.show": () => m.plugin_interpreter_tool_show(),
  },
};

export default ui;
