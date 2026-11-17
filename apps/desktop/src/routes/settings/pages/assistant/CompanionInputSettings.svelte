<script lang="ts">
  // 「交流 → 你 → 琴音」：陪玩热键与交互方式（原在「琴音陪玩 → 设置」，ADR 023 搬来），下面接快捷指令等。
  import { updateSettings } from "../../../../lib/ipc";
  import { settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import AssistantInteraction from "./AssistantInteraction.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  const config = $derived($settingsStore?.assistant);
</script>

{#if config}
  <div class="space-y-6" data-testid="assistant-settings">
    <SettingsSection title="陪玩热键" id="assistant-hotkey-title" description="在游戏里按下后说一句话，交给琴音。">
      <SettingsRow label="陪玩热键" description="在游戏里按下后说一句话，交给琴音。" stacked>
        {#snippet control()}
          <HotkeyField id="assistant" title="陪玩热键" value={config.hotkey} placeholder="如 Ctrl+F9" compact
            onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: { hotkey } })); }} />
        {/snippet}
      </SettingsRow>
      <AssistantInteraction />
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
