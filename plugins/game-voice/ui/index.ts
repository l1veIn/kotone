// 游戏语音插件的界面入口（ADR 020、038）：图标、插件页面（左侧栏「插件 → 游戏语音」）、她用 ui.open 打开的右侧栏页面、
// 聊天里工具那一行的名字、推送“她对队友说了”的卡片（ADR 043 D11：聊天里的卡片和点开以后右侧栏里是同一个组件）。
// 宿主在 apps/desktop/src/lib/pluginUi.ts 里登记。工具名沿用搬进插件以前的 `voice_chat.*`。
import { AudioLines } from "@lucide/svelte";
import type { PluginUi } from "$lib/pluginUi";
import { m } from "$lib/paraglide/messages.js";
import GameVoicePage from "./GameVoicePage.svelte";
import GameVoiceSettings from "./GameVoiceSettings.svelte";
import GameVoiceView from "./GameVoiceView.svelte";
import SaidCard from "./SaidCard.svelte";
import { PAGE_HOME, PAGE_SAID } from "./voiceChat";

const ui: PluginUi = {
  icon: AudioLines,
  page: GameVoicePage,
  settings: GameVoiceSettings,
  views: {
    [PAGE_HOME]: { page: GameVoiceView, title: () => m.plugin_game_voice_name() },
    [PAGE_SAID]: { card: SaidCard, page: SaidCard, title: () => m.plugin_game_voice_push_said_title() },
  },
  toolLabels: {
    "voice_chat.status": () => m.plugin_game_voice_tool_status(),
    "voice_chat.install": () => m.plugin_game_voice_tool_install(),
    "voice_chat.default_mic": () => m.plugin_game_voice_tool_default_mic(),
    "voice_chat.say": () => m.plugin_game_voice_tool_say(),
    "voice_chat.test": () => m.plugin_game_voice_tool_test(),
  },
};

export default ui;
