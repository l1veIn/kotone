<script lang="ts">
  /*
   * 语音输入插件的 App（ADR 038）：原来左侧栏「语音输入」那一整块（输入 / 文字处理 / 历史三页，悬浮窗在“输入”里）
   * 原样搬进插件，从左侧栏「插件 → 语音输入」打开。页面 ID 和分组锚点不变（general / processing / history，
   * voice-asr / voice-overlay……），「去看看」的位置（「语音输入 → 悬浮窗」等）经 `open.params` 落到这里的分页和分组。
   * 插件开关在最上面；关着时语音输入热键、悬浮窗都没有（宿主按开关处理），这里只留开关和一句说明。
   * 她替你在游戏聊天里发言（game_chat.send）是本体的，不跟着这个开关（ADR 039 D2）。
   */
  import { tick } from "svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { pluginEnabled } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { companionBindings, usesLegacyVoiceInput } from "$lib/ipc/contracts";
  import { openTarget, type VoiceInputPage } from "$lib/stores/navigation";
  import { pendingPrompt } from "$lib/agentChat/pendingPrompt";
  import { settingsStore } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import VoiceInputShell from "./VoiceInputShell.svelte";
  import GeneralPage from "./GeneralPage.svelte";
  import TextProcessingPage from "./TextProcessingPage.svelte";
  import HistoryPage from "./HistoryPage.svelte";

  let { plugin, open = null }: PluginPageProps = $props();

  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const TABS: VoiceInputPage[] = ["general", "processing", "history"];
  let tab = $state<VoiceInputPage>("general");
  /** 语音输入的四步引导（V 路径）展开着：新用户引导选了“只用语音输入”时打开。 */
  let guideOpen = $state(false);

  // 从「去看看」、引导、标题栏的“启动识别”打开：切到那一页，滚到那一组
  $effect(() => {
    const params = open?.params;
    if (!params) return;
    const wanted = params.tab as VoiceInputPage | undefined;
    tab = wanted && TABS.includes(wanted) ? wanted : "general";
    if (params.guide === true) guideOpen = true;
    const anchor = typeof params.anchor === "string" ? params.anchor : null;
    if (anchor) void tick().then(() => requestAnimationFrame(() => document.getElementById(anchor)?.scrollIntoView({ block: "start" })));
  });

  /** 「让 Kotone 帮我配置」：她连好了就请她帮忙（只预填，不经过语音输入）；还没连就先填 Key。 */
  function configure() {
    const s = $settingsStore;
    const connectionId = companionBindings(s?.assistant)?.connectionId?.trim();
    if (s && !usesLegacyVoiceInput(s.ui) && connectionId && s.connections.some((c) => c.id === connectionId)) {
      pendingPrompt.set(m.shell_settings_voice_setup_prompt());
      openTarget({ page: "assistant", section: "chat" });
    } else {
      openTarget({ page: "setup-key" });
    }
  }
</script>

{#if on}
  <VoiceInputShell page={tab} onTab={(next: VoiceInputPage) => (tab = next)} onConfigure={configure}>
    <div class="mx-auto w-full max-w-3xl px-6 pt-4">
      <SettingsSection testid="plugin-voice-input-switch">
        <PluginSwitches {plugin} />
      </SettingsSection>
    </div>
    {#if tab === "general"}
      <GeneralPage {guideOpen} onOpenGuide={() => (guideOpen = true)} onCloseGuide={() => (guideOpen = false)}
        onConfigure={configure} onOpenModels={() => openTarget({ page: "advanced", tab: "models", anchor: "models-asr" })}
        onOpenSound={() => openTarget({ page: "advanced", tab: "sound" })} onOpenGames={() => openTarget({ page: "game" })} />
    {:else if tab === "processing"}
      <TextProcessingPage onOpenConnections={() => openTarget({ page: "advanced", tab: "connections" })} />
    {:else}
      <HistoryPage />
    {/if}
  </VoiceInputShell>
{:else}
  <div class="ui-scope mx-auto w-full max-w-3xl space-y-2.5 px-6 pt-7 text-foreground" data-testid="voice-input">
    <SettingsSection testid="plugin-voice-input-switch">
      <PluginSwitches {plugin} />
    </SettingsSection>
    <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-voice-input-off">{m.plugin_voice_input_off_note()}</p>
  </div>
{/if}
