// 语音输入插件的界面入口（ADR 038）：图标和插件的 App（左侧栏「插件 → 语音输入」：原来左侧栏「语音输入」的
// 输入 / 文字处理 / 历史三页）。她的 game_chat.send 在聊天里的名字沿用 lib/agent.ts 里的那一条（工具名没变）。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Mic } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import VoiceInputApp from "./VoiceInputApp.svelte";

const ui: PluginUi = {
  icon: Mic,
  page: VoiceInputApp,
};

export default ui;
