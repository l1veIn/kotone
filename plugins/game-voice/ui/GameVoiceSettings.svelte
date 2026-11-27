<script lang="ts">
  import { onMount } from "svelte";
  import { Mic, RotateCcw } from "@lucide/svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { listAudioDevices, type AudioDevice } from "$lib/ipc";
  import { pluginEnabled } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { errText, patchSettings, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import {
    PLUGIN_ID, voiceChatDefaultMic, voiceChatPrefs, voiceChatStatus,
    type VoiceChatPrefs, type VoiceChatStatus,
  } from "./voiceChat";

  let { plugin }: PluginPageProps = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const config = $derived(voiceChatPrefs($settingsStore));
  let status = $state<VoiceChatStatus | null>(null);
  let devices = $state<AudioDevice[]>([]);
  let savingDefault = $state(false);
  let version = 0;
  const gainOptions = $derived([...new Set([50, 75, 100, 150, 200, config.micGain])].sort((a, b) => a - b)
    .map((value) => ({ value: String(value), label: value + "%" })));
  const micOptions = $derived(devices.filter((d) => !d.name.startsWith("CABLE Output")).map((d) => ({ value: d.id, label: d.name })));

  async function refresh() {
    const request = ++version;
    if (!on) { status = null; return; }
    try {
      const next = await voiceChatStatus();
      if (request === version) status = next;
    } catch (error) {
      if (request === version) toast(false, m.plugin_game_voice_status_failed({ error: errText(error) }));
    }
  }
  onMount(() => {
    void listAudioDevices().then((list) => (devices = list)).catch(() => {});
  });
  $effect(() => {
    void refresh();
    return () => { version++; };
  });

  async function save(patch: Partial<VoiceChatPrefs>, message: string) {
    await patchSettings({ plugins: { [PLUGIN_ID]: { settings: patch } } }, message);
  }
  async function setDefault(use: boolean) {
    savingDefault = true;
    try {
      const r = await voiceChatDefaultMic(use);
      toast(true, use
        ? m.plugin_game_voice_default_mic_set({ mic: String(r.defaultMic) })
        : m.plugin_game_voice_default_mic_restored({ mic: r.defaultMic ?? m.plugin_game_voice_original_mic() }));
    } catch (error) {
      toast(false, errText(error));
    } finally {
      savingDefault = false;
      void refresh();
    }
  }
</script>

<SettingsPage title={m.plugin_game_voice_devices()} testid="game-voice-device-settings">
  <SettingsSection><PluginSwitches {plugin} /></SettingsSection>
  {#if on}
    <SettingsSection title={m.plugin_game_voice_mic_label()} testid="voice-chat-settings">
      <SettingsRow label={m.plugin_game_voice_include_mic_label()} description={m.plugin_game_voice_include_mic_desc()}>
        {#snippet control()}
          <Switch data-testid="voice-chat-include-mic" bind:checked={() => config.includeMic, (includeMic) => void save({ includeMic }, m.plugin_game_voice_saved())} aria-label={m.plugin_game_voice_include_mic_label()} />
        {/snippet}
      </SettingsRow>
      {#if config.includeMic}
        <SettingsRow label={m.plugin_game_voice_mic_label()} description={m.plugin_game_voice_mic_desc()} stacked>
          {#snippet control()}
            <div class="flex flex-wrap gap-2">
              <SelectField class="min-w-0 flex-1" ariaLabel={m.plugin_game_voice_mic_label()} testid="voice-chat-mic" value={config.micDeviceId} options={micOptions}
                onchange={(id) => void save({ micDeviceId: id }, m.plugin_game_voice_mic_saved())} />
              <SelectField class="w-24" ariaLabel={m.plugin_game_voice_mic_gain_aria()} value={String(config.micGain)} options={gainOptions}
                onchange={(v) => void save({ micGain: Number(v) }, m.plugin_game_voice_mic_gain_saved())} />
            </div>
          {/snippet}
        </SettingsRow>
      {/if}
      <SettingsRow label={m.plugin_game_voice_default_mic_label()} description={m.plugin_game_voice_default_mic_desc({ mic: status?.windowsDefaultMic?.name ?? m.plugin_game_voice_unknown() })}>
        {#snippet control()}
          {#if status?.windowsDefaultIsCable}
            <Button size="sm" variant="outline" data-testid="voice-chat-restore-mic" disabled={savingDefault || !status.canRestoreDefaultMic} onclick={() => void setDefault(false)}>
              <RotateCcw class="size-3.5" />{m.plugin_game_voice_restore_mic()}
            </Button>
          {:else}
            <Button size="sm" variant="outline" data-testid="voice-chat-use-cable" disabled={savingDefault || !status?.driverInstalled} onclick={() => void setDefault(true)}>
              <Mic class="size-3.5" />{m.plugin_game_voice_use_cable()}
            </Button>
          {/if}
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    <p class="text-xs leading-relaxed text-muted-foreground">{m.plugin_game_voice_switch_desc()}</p>
  {/if}
</SettingsPage>
