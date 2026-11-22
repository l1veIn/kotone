<script lang="ts">
  // 「琴音陪玩 → 设置 → 游戏语音」（ADR 020）：琴音用虚拟麦克风 VB-CABLE 替你在游戏语音里说话。
  // 开关等设置在 config.json `voiceChat`；安装、改默认麦克风、测试走 voice_chat_* 命令（与工具同一实现）。
  import { onMount } from "svelte";
  import { Download, LoaderCircle, Mic, RotateCcw, Volume2 } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { listAudioDevices, type AudioDevice, type VoiceChatConfig } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import {
    voiceChatDefaultMic, voiceChatInstall, voiceChatSay, voiceChatStatus, voiceChatTest, type VoiceChatStatus,
  } from "../../../../lib/voiceChat";

  const config = $derived<VoiceChatConfig>($settingsStore?.voiceChat ?? {
    enabled: false, includeMic: true, micDeviceId: "default", micGain: 100,
  });
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
    try {
      status = await voiceChatStatus(config.enabled);
    } catch (error) {
      toast(false, m.companion_voice_chat_status_failed({ error: errText(error) }));
    }
  }

  onMount(() => {
    void refresh();
    void listAudioDevices().then((list) => (devices = list)).catch(() => {});
    const timer = setInterval(() => void refresh(), 3000);
    return () => clearInterval(timer);
  });

  async function save(patch: Partial<VoiceChatConfig>, message: string) {
    await patchSettings({ voiceChat: { ...config, ...patch } }, message);
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
    toast(r.installed, r.installed ? m.companion_voice_chat_installed() : (r.note ?? m.companion_voice_chat_installer_ran())));
  const useCable = (use: boolean) => run("mic", () => voiceChatDefaultMic(use), (r) =>
    toast(true, use
      ? m.companion_voice_chat_default_mic_set({ mic: String(r.defaultMic) })
      : m.companion_voice_chat_default_mic_restored({ mic: r.defaultMic ?? m.companion_voice_chat_original_mic() })));
  const test = () => run("test", voiceChatTest, (r) => toast(r.ok, r.note));
  const say = () => run("say", () => voiceChatSay(m.companion_voice_chat_test_phrase()), () => toast(true, m.companion_voice_chat_said()));

  const stateText = $derived.by(() => {
    if (!status) return m.companion_voice_chat_state_loading();
    if (status.installing) return m.companion_voice_chat_state_installing();
    if (!status.driverInstalled) return m.companion_voice_chat_state_not_installed();
    if (!config.enabled) return m.companion_voice_chat_state_disabled();
    if (status.error) return m.companion_voice_chat_state_error({ error: status.error });
    return status.running ? m.companion_voice_chat_state_running() : m.companion_voice_chat_state_starting();
  });
</script>

<SettingsSection title={m.companion_voice_chat_title()} description={m.companion_voice_chat_desc()} id="voice-chat-settings">
  <div data-testid="voice-chat-settings" class="contents">
    <SettingsRow label={m.companion_voice_chat_virtual_mic()} description={status?.attribution ?? ""}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <span class="text-xs text-muted-foreground" data-testid="voice-chat-state">{stateText}</span>
          {#if status && !status.driverInstalled}
            <Button size="sm" data-testid="voice-chat-install" disabled={busy !== ""} onclick={() => void install()}>
              {#if busy === "install"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Download class="size-3.5" />{/if}{m.companion_voice_chat_install()}
            </Button>
          {/if}
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_voice_chat_enabled_label()} description={m.companion_voice_chat_enabled_desc()}>
      {#snippet control()}
        <Switch data-testid="voice-chat-enabled" disabled={!status?.driverInstalled}
          bind:checked={() => config.enabled, (checked) => void save({ enabled: checked }, checked ? m.companion_voice_chat_enabled_on() : m.companion_voice_chat_enabled_off())} aria-label={m.companion_voice_chat_enabled_label()} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_voice_chat_include_mic_label()} description={m.companion_voice_chat_include_mic_desc()}>
      {#snippet control()}
        <Switch data-testid="voice-chat-include-mic" bind:checked={() => config.includeMic, (checked) => void save({ includeMic: checked }, m.companion_saved())} aria-label={m.companion_voice_chat_include_mic_label()} />
      {/snippet}
    </SettingsRow>
    {#if config.includeMic}
      <SettingsRow label={m.companion_voice_chat_mic_label()} description={m.companion_voice_chat_mic_desc()}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <div class="w-56 max-w-full">
              <SelectField ariaLabel={m.companion_voice_chat_mic_label()} testid="voice-chat-mic" value={config.micDeviceId || "default"} options={micOptions}
                onchange={(id) => void save({ micDeviceId: id }, m.companion_voice_chat_mic_saved())} />
            </div>
            <div class="w-24">
              <SelectField ariaLabel={m.companion_voice_chat_mic_gain_aria()} value={String(config.micGain)} options={gainOptions}
                onchange={(v) => void save({ micGain: Number(v) }, m.companion_voice_chat_mic_gain_saved())} />
            </div>
          </div>
        {/snippet}
      </SettingsRow>
    {/if}
    <SettingsRow label={m.companion_voice_chat_default_mic_label()} description={m.companion_voice_chat_default_mic_desc({ mic: status?.windowsDefaultMic?.name ?? m.companion_voice_chat_unknown() })}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          {#if status?.windowsDefaultIsCable}
            <Button size="sm" variant="outline" data-testid="voice-chat-restore-mic" disabled={busy !== "" || !status?.canRestoreDefaultMic} onclick={() => void useCable(false)}>
              <RotateCcw class="size-3.5" />{m.companion_voice_chat_restore_mic()}
            </Button>
          {:else}
            <Button size="sm" variant="outline" data-testid="voice-chat-use-cable" disabled={busy !== "" || !status?.driverInstalled} onclick={() => void useCable(true)}>
              <Mic class="size-3.5" />{m.companion_voice_chat_use_cable()}
            </Button>
          {/if}
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_voice_chat_try_label()} description={status && !status.ttsCanSpeak ? m.companion_voice_chat_try_desc_system() : m.companion_voice_chat_try_desc()}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <Button size="sm" variant="outline" data-testid="voice-chat-test" disabled={busy !== "" || !status?.running} onclick={() => void test()}>
            {#if busy === "test"}<LoaderCircle class="size-3.5 animate-spin" />{/if}{m.companion_voice_chat_test_tone()}
          </Button>
          <Button size="sm" variant="outline" data-testid="voice-chat-say" disabled={busy !== "" || !status?.running || !status?.ttsCanSpeak} onclick={() => void say()}>
            <Volume2 class="size-3.5" />{m.companion_voice_chat_say()}
          </Button>
        </div>
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
