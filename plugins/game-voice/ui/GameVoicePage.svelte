<script lang="ts">
  // 游戏语音插件的页面（ADR 020、038）：她用虚拟麦克风 VB-CABLE 替你在游戏语音里说话。
  // 左侧栏「插件 → 游戏语音」和她 `ui.open` 打开的右侧栏页面共用（右侧栏里不重复页标题）。
  // 插件开关就是转发的开关（`plugins.game-voice.enabled`）；关着时只显示开关和一句说明。
  // 开着时：装没装 VB-CABLE（自动安装）、混入自己的麦克风、选麦克风和音量（`plugins.game-voice.settings`）、
  // 改 / 改回 Windows 默认麦克风、测试音和说一句——都经插件工具 voice_chat.*（和她用的是同一个实现）。
  import { onMount } from "svelte";
  import { Download, LoaderCircle, Mic, RotateCcw, Volume2 } from "@lucide/svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { listAudioDevices, type AudioDevice } from "$lib/ipc";
  import { pluginDescription, pluginEnabled, pluginName } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { errText, patchSettings, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import {
    PLUGIN_ID, voiceChatDefaultMic, voiceChatInstall, voiceChatPrefs, voiceChatSay, voiceChatStatus, voiceChatTest,
    type VoiceChatPrefs, type VoiceChatStatus,
  } from "./voiceChat";

  let { plugin, embedded = false }: PluginPageProps & {
    /** 在右侧栏里（她用 ui.open 打开）：不要大标题。 */
    embedded?: boolean;
  } = $props();

  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const config = $derived<VoiceChatPrefs>(voiceChatPrefs($settingsStore));
  let status = $state<VoiceChatStatus | null>(null);
  let devices = $state<AudioDevice[]>([]);
  let busy = $state<"" | "install" | "mic" | "test" | "say">("");

  const gains = [
    { value: "50", label: "50%" }, { value: "75", label: "75%" }, { value: "100", label: "100%" },
    { value: "150", label: "150%" }, { value: "200", label: "200%" },
  ];
  const gainOptions = $derived(
    gains.some((g) => g.value === String(config.micGain)) ? gains : [...gains, { value: String(config.micGain), label: `${config.micGain}%` }],
  );
  const micOptions = $derived(
    devices
      .filter((d) => !d.name.startsWith("CABLE Output"))
      .map((d) => ({ value: d.id, label: d.name })),
  );

  async function refresh() {
    if (!on) {
      status = null;
      return;
    }
    try {
      status = await voiceChatStatus();
    } catch (error) {
      toast(false, m.plugin_game_voice_status_failed({ error: errText(error) }));
    }
  }

  onMount(() => {
    void listAudioDevices().then((list) => (devices = list)).catch(() => {});
    const timer = setInterval(() => void refresh(), 3000);
    return () => clearInterval(timer);
  });

  // 打开 / 关掉插件时马上刷新（转发在后台开关，稍后再读一次）
  $effect(() => {
    if (on) {
      void refresh();
      const later = setTimeout(() => void refresh(), 600);
      return () => clearTimeout(later);
    }
    status = null;
  });

  async function save(patch: Partial<VoiceChatPrefs>, message: string) {
    await patchSettings({ plugins: { [PLUGIN_ID]: { settings: patch } } }, message);
    setTimeout(() => void refresh(), 600);
  }

  async function run<T>(kind: typeof busy, work: () => Promise<T>, done: (value: T) => void) {
    busy = kind;
    try {
      done(await work());
    } catch (error) {
      toast(false, errText(error));
    } finally {
      busy = "";
      void refresh();
    }
  }

  const install = () => run("install", voiceChatInstall, (r) =>
    toast(r.installed, r.installed ? m.plugin_game_voice_installed() : (r.note ?? m.plugin_game_voice_installer_ran())));
  const useCable = (use: boolean) => run("mic", () => voiceChatDefaultMic(use), (r) =>
    toast(true, use
      ? m.plugin_game_voice_default_mic_set({ mic: String(r.defaultMic) })
      : m.plugin_game_voice_default_mic_restored({ mic: r.defaultMic ?? m.plugin_game_voice_original_mic() })));
  const test = () => run("test", voiceChatTest, (r) => toast(r.ok, r.note));
  const say = () => run("say", () => voiceChatSay(m.plugin_game_voice_test_phrase()), () => toast(true, m.plugin_game_voice_said()));

  const stateText = $derived.by(() => {
    if (!status) return m.plugin_game_voice_state_loading();
    if (status.installing) return m.plugin_game_voice_state_installing();
    if (!status.driverInstalled) return m.plugin_game_voice_state_not_installed();
    if (status.error) return m.plugin_game_voice_state_error({ error: status.error });
    return status.running ? m.plugin_game_voice_state_running() : m.plugin_game_voice_state_starting();
  });
</script>

{#snippet body()}
  <div class="space-y-2.5">
    <SettingsSection testid="plugin-game-voice-switch">
      <PluginSwitches {plugin} />
    </SettingsSection>
    <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-game-voice-note">{m.plugin_game_voice_switch_desc()}</p>
  </div>
  {#if !on}
    <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-game-voice-off">{m.plugin_game_voice_off_note()}</p>
  {:else}
    <SettingsSection title={m.plugin_game_voice_section_title()} description={m.plugin_game_voice_section_desc()} id="voice-chat-settings" testid="voice-chat-settings">
      <SettingsRow label={m.plugin_game_voice_virtual_mic()} description={status?.attribution ?? ""}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <span class="text-xs text-muted-foreground" data-testid="voice-chat-state">{stateText}</span>
            {#if status && !status.driverInstalled}
              <Button size="sm" data-testid="voice-chat-install" disabled={busy !== ""} onclick={() => void install()}>
                {#if busy === "install"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Download class="size-3.5" />{/if}{m.plugin_game_voice_install()}
              </Button>
            {/if}
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.plugin_game_voice_include_mic_label()} description={m.plugin_game_voice_include_mic_desc()}>
        {#snippet control()}
          <Switch data-testid="voice-chat-include-mic" bind:checked={() => config.includeMic, (checked) => void save({ includeMic: checked }, m.plugin_game_voice_saved())} aria-label={m.plugin_game_voice_include_mic_label()} />
        {/snippet}
      </SettingsRow>
      {#if config.includeMic}
        <SettingsRow label={m.plugin_game_voice_mic_label()} description={m.plugin_game_voice_mic_desc()}>
          {#snippet control()}
            <div class="flex items-center gap-2">
              <div class="w-56 max-w-full">
                <SelectField ariaLabel={m.plugin_game_voice_mic_label()} testid="voice-chat-mic" value={config.micDeviceId || "default"} options={micOptions}
                  onchange={(id) => void save({ micDeviceId: id }, m.plugin_game_voice_mic_saved())} />
              </div>
              <div class="w-24">
                <SelectField ariaLabel={m.plugin_game_voice_mic_gain_aria()} value={String(config.micGain)} options={gainOptions}
                  onchange={(v) => void save({ micGain: Number(v) }, m.plugin_game_voice_mic_gain_saved())} />
              </div>
            </div>
          {/snippet}
        </SettingsRow>
      {/if}
      <SettingsRow label={m.plugin_game_voice_default_mic_label()} description={m.plugin_game_voice_default_mic_desc({ mic: status?.windowsDefaultMic?.name ?? m.plugin_game_voice_unknown() })}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            {#if status?.windowsDefaultIsCable}
              <Button size="sm" variant="outline" data-testid="voice-chat-restore-mic" disabled={busy !== "" || !status?.canRestoreDefaultMic} onclick={() => void useCable(false)}>
                <RotateCcw class="size-3.5" />{m.plugin_game_voice_restore_mic()}
              </Button>
            {:else}
              <Button size="sm" variant="outline" data-testid="voice-chat-use-cable" disabled={busy !== "" || !status?.driverInstalled} onclick={() => void useCable(true)}>
                <Mic class="size-3.5" />{m.plugin_game_voice_use_cable()}
              </Button>
            {/if}
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.plugin_game_voice_try_label()} description={status && !status.ttsCanSpeak ? m.plugin_game_voice_try_desc_system() : m.plugin_game_voice_try_desc()}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <Button size="sm" variant="outline" data-testid="voice-chat-test" disabled={busy !== "" || !status?.running} onclick={() => void test()}>
              {#if busy === "test"}<LoaderCircle class="size-3.5 animate-spin" />{/if}{m.plugin_game_voice_test_tone()}
            </Button>
            <Button size="sm" variant="outline" data-testid="voice-chat-say" disabled={busy !== "" || !status?.running || !status?.ttsCanSpeak} onclick={() => void say()}>
              <Volume2 class="size-3.5" />{m.plugin_game_voice_say()}
            </Button>
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>
  {/if}
{/snippet}

{#if embedded}
  <div class="space-y-6" data-testid="plugin-page-game-voice">{@render body()}</div>
{:else}
  <SettingsPage title={pluginName(plugin.manifest)} description={pluginDescription(plugin.manifest)} testid="plugin-page-game-voice">
    <div class="space-y-6">{@render body()}</div>
  </SettingsPage>
{/if}
