// 语音输入插件的界面入口（ADR 038）：图标和插件的 App（左侧栏「插件 → 语音输入」：原来左侧栏「语音输入」的
// 输入 / 文字处理 / 历史三页）。她的 game_chat.send 在聊天里的名字沿用 lib/agent.ts 里的那一条（工具名没变）。
// 推送的两张卡片（ADR 043 D10）：“发出去的话”“没发出去”，聊天里的卡片和点开以后右侧栏里是同一个组件。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。
import { Mic } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import VoiceInputApp from "./VoiceInputApp.svelte";
import VoiceInputSettings from "./VoiceInputSettings.svelte";
import SentCard from "./SentCard.svelte";
import FailedCard from "./FailedCard.svelte";
import { PAGE_FAILED, PAGE_SENT } from "./pushes";

const ui: PluginUi = {
  icon: Mic,
  page: VoiceInputApp,
  settings: VoiceInputSettings,
  views: {
    [PAGE_SENT]: { card: SentCard, page: SentCard, title: () => m.plugin_voice_input_push_sent_title() },
    [PAGE_FAILED]: { card: FailedCard, page: FailedCard, title: () => m.plugin_voice_input_push_failed_title() },
  },
};

export default ui;
