<script lang="ts">
  // 语音输入的其他热键（ADR 028 D12：从原「设置 → 其他热键」搬来）。这些键是插件登记的命令，
  // 只在「设置 → 热键」改（ADR 040 D1、D3），这里显示现在的键和“去改”。
  import { settingsStore } from "$lib/stores/ui";
  import HotkeyLink from "$lib/components/HotkeyLink.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { m } from "$lib/paraglide/messages.js";

  type Key = "channelCycleHotkey" | "resendLastHotkey" | "togglePostProcessingHotkey" | "cyclePostProcessingHotkey";
  /** 插件清单里登记的命令（`plugin.toml` 的 `[[commands]]`），id 是热键表里的行 id。 */
  const HOTKEYS: { id: string; title: string; desc: string; key: Key }[] = [
    { id: "voice-input.channel-cycle", title: m.voice_hotkey_name_channel_cycle(), key: "channelCycleHotkey", desc: m.voice_hotkeys_channel_cycle_desc() },
    { id: "voice-input.resend-last", title: m.voice_hotkey_name_resend_last(), key: "resendLastHotkey", desc: m.voice_hotkeys_resend_last_desc() },
    { id: "voice-input.post-process-toggle", title: m.voice_hotkey_name_post_process_toggle(), key: "togglePostProcessingHotkey", desc: m.voice_hotkeys_post_process_toggle_desc() },
    { id: "voice-input.post-process-cycle", title: m.voice_hotkey_name_post_process_cycle(), key: "cyclePostProcessingHotkey", desc: m.voice_hotkeys_post_process_cycle_desc() },
  ];
</script>

<div id="voice-hotkeys" class="scroll-mt-16">
  <SettingsSection title={m.voice_hotkeys_section_title()} description={m.voice_hotkeys_section_desc()} testid="voice-hotkeys">
    {#each HOTKEYS as item (item.id)}
      <SettingsRow label={item.title} description={item.desc} stacked testid={`hotkey-section-${item.id}`}>
        {#snippet control()}
          <HotkeyLink id={item.id} value={$settingsStore?.[item.key] ?? ""} />
        {/snippet}
      </SettingsRow>
    {/each}
  </SettingsSection>
</div>
