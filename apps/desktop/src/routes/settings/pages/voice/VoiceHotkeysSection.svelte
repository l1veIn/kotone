<script lang="ts">
  // 语音输入的其他热键（ADR 028 D12：从原「设置 → 其他热键」搬来）。
  // 启动 / 停止本地语音识别的快捷键在识别卡片里，停止热键在「设置 → 系统 → 停下」。
  import { updateSettings } from "../../../../lib/ipc";
  import { settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { m } from "$lib/paraglide/messages.js";

  type Key = "channelCycleHotkey" | "resendLastHotkey" | "togglePostProcessingHotkey" | "cyclePostProcessingHotkey";
  /** 与后端 KNOWN_NAMED_HOTKEYS 对齐。 */
  const HOTKEYS: { id: string; title: string; desc: string; key: Key; placeholder: string }[] = [
    { id: "channel-cycle", title: m.voice_hotkey_name_channel_cycle(), key: "channelCycleHotkey", placeholder: m.voice_hotkey_placeholder({ example: "Shift+CapsLock" }),
      desc: m.voice_hotkeys_channel_cycle_desc() },
    { id: "resend-last", title: m.voice_hotkey_name_resend_last(), key: "resendLastHotkey", placeholder: m.voice_hotkey_placeholder({ example: "Alt+F6" }),
      desc: m.voice_hotkeys_resend_last_desc() },
    { id: "post-process-toggle", title: m.voice_hotkey_name_post_process_toggle(), key: "togglePostProcessingHotkey", placeholder: m.voice_hotkey_placeholder({ example: "Alt+F7" }),
      desc: m.voice_hotkeys_post_process_toggle_desc() },
    { id: "post-process-cycle", title: m.voice_hotkey_name_post_process_cycle(), key: "cyclePostProcessingHotkey", placeholder: m.voice_hotkey_placeholder({ example: "Alt+F8" }),
      desc: m.voice_hotkeys_post_process_cycle_desc() },
  ];
</script>

<div id="voice-hotkeys" class="scroll-mt-16">
  <SettingsSection title={m.voice_hotkeys_section_title()} description={m.voice_hotkeys_section_desc()} testid="voice-hotkeys">
    {#each HOTKEYS as item (item.id)}
      <SettingsRow label={item.title} description={item.desc} stacked testid={`hotkey-section-${item.id}`}>
        {#snippet control()}
          <HotkeyField id={item.id} title={item.title} value={$settingsStore?.[item.key] ?? ""} placeholder={item.placeholder} compact
            onSave={async (key) => { settingsStore.set(await updateSettings({ [item.key]: key })); }} />
        {/snippet}
      </SettingsRow>
    {/each}
  </SettingsSection>
</div>
