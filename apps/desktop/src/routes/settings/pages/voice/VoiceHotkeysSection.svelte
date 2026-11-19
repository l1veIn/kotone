<script lang="ts">
  // 语音输入的其他热键（ADR 028 D12：从原「设置 → 其他热键」搬来）。
  // 启动 / 停止本地语音识别的快捷键在识别卡片里，停止热键在「设置 → 系统 → 停下」。
  import { updateSettings } from "../../../../lib/ipc";
  import { settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";

  type Key = "channelCycleHotkey" | "resendLastHotkey" | "togglePostProcessingHotkey" | "cyclePostProcessingHotkey";
  /** 与后端 KNOWN_NAMED_HOTKEYS 对齐。 */
  const HOTKEYS: { id: string; title: string; desc: string; key: Key; placeholder: string }[] = [
    { id: "channel-cycle", title: "频道切换热键", key: "channelCycleHotkey", placeholder: "如 Shift+CapsLock",
      desc: "支持多频道的游戏（如英雄联盟的「队伍 / 所有人」）按顺序切换频道，悬浮窗会显示当前频道。" },
    { id: "resend-last", title: "重发最近一条热键", key: "resendLastHotkey", placeholder: "如 Alt+F6",
      desc: "把最近一条发送成功的话重新发到当前窗口（正在说话时按下不生效）。" },
    { id: "post-process-toggle", title: "文字处理开关热键", key: "togglePostProcessingHotkey", placeholder: "如 Alt+F7",
      desc: "快速打开 / 关闭文字处理，对下一句生效；状态会短暂显示在悬浮窗。" },
    { id: "post-process-cycle", title: "切换文字处理流程热键", key: "cyclePostProcessingHotkey", placeholder: "如 Alt+F8",
      desc: "切换到下一条文字处理流程；只有一条流程时不生效。" },
  ];
</script>

<div id="voice-hotkeys" class="scroll-mt-16">
  <SettingsSection title="语音输入的其他热键" description="按下组合键录入；留空 = 关闭，不会误触发。" testid="voice-hotkeys">
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
