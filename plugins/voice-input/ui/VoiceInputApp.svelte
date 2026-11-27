<script lang="ts">
  /*
   * 语音输入插件的 App（ADR 038）：原来左侧栏「语音输入」那一整块（输入 / 文字处理 / 历史三页，悬浮窗在“输入”里）
   * 原样搬进插件，从左侧栏「插件 → 语音输入」打开。页面 ID 和分组锚点不变（general / processing / history，
   * voice-asr / voice-overlay……），「去看看」的位置（「语音输入 → 悬浮窗」等）经 `open.params` 落到这里的分页和分组。
   * 插件设置里保留启用开关；关着时主页也提供打开入口。
   * 她替你在游戏聊天里发言（game_chat.send）是本体的，不跟着这个开关（ADR 039 D2）。
   */
  import { tick } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { pluginEnabled, pluginName, setPluginEnabled } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { companionBindings, usesLegacyVoiceInput } from "$lib/ipc/contracts";
  import { openTarget, type VoiceInputPage } from "$lib/stores/navigation";
  import { pendingPrompt } from "$lib/agentChat/pendingPrompt";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
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
  let optionsOpen = $state(false);
  let enabling = $state(false);

  async function enable() {
    if (enabling) return;
    enabling = true;
    try { await setPluginEnabled(plugin.manifest.id, true); }
    catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
    finally { enabling = false; }
  }

  // 从「去看看」、引导、标题栏的“启动识别”打开：切到那一页，滚到那一组
  $effect(() => {
    const params = open?.params;
    if (!params) return;
    const wanted = params.tab as VoiceInputPage | undefined;
    tab = wanted && TABS.includes(wanted) ? wanted : "general";
    if (params.guide === true) guideOpen = true;
    const anchor = typeof params.anchor === "string" ? params.anchor : null;
    if (anchor?.startsWith("voice-") && anchor !== "voice-asr") optionsOpen = true;
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
    {#if tab === "general"}
      <GeneralPage {guideOpen} bind:optionsOpen onOpenGuide={() => (guideOpen = true)} onCloseGuide={() => (guideOpen = false)}
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
    <h1 class="m-0 text-2xl font-semibold tracking-tight">{m.voice_shell_title()}</h1>
    <p class="m-0 pb-3 text-sm text-muted-foreground" data-testid="plugin-voice-input-off">{m.plugin_voice_input_off_note()}</p>
    <Button disabled={enabling} data-testid="voice-input-enable" onclick={() => void enable()}>{m.system_plugins_enable({ name: pluginName(plugin.manifest) })}</Button>
  </div>
{/if}
