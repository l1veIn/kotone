<script lang="ts">
  // 「琴音 → 设置 → 呼叫」：呼叫琴音的热键（按一次就听，相当于按键版的语音唤醒），下面接快捷指令等。
  import { companionName } from "$lib/stores/companion";
  import { companionBindingPatch, companionBindings, updateSettings } from "../../../../lib/ipc";
  import { settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  const config = $derived($settingsStore?.assistant);
  const bindings = $derived(companionBindings(config));
  const name = $derived($companionName);
</script>

{#if bindings}
  <div class="space-y-6" data-testid="call-settings">
    <SettingsSection title={`呼叫${name}`} id="assistant-hotkey-title" description={`按一下就在右下角气泡里听你说，说完自动交给${name}；在听时再按一下立刻提交。`}>
      <SettingsRow label="呼叫热键" description="默认是 ~ 键（Esc 下面那个）。清空 = 不用热键，只用按钮或唤醒词。" stacked>
        {#snippet control()}
          <HotkeyField id="assistant" title="呼叫热键" value={bindings.hotkey} placeholder="如 ~ 或 F9" compact
            onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: companionBindingPatch(config, { hotkey }) })); }} />
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
