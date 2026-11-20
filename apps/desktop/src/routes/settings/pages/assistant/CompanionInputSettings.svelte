<script lang="ts">
  // 「设置 → 怎么叫她」：呼叫键（按一次就听，相当于按键版的语音唤醒）、回答完接着听（ADR 029），
  // 下面接快捷指令等（ADR 028）。
  import { companionName } from "$lib/stores/companion";
  import { companionBindingPatch, companionBindings, updateSettings } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  const config = $derived($settingsStore?.assistant);
  const bindings = $derived(companionBindings(config));
  const name = $derived($companionName);
  /** 回答完接着听几秒（新字段 assistant.followUpSeconds，旧配置没有 = 8 秒）。 */
  const followUp = $derived(config?.followUpSeconds ?? 8);
  const followUpOptions = [
    { value: "0", label: "不接着听" }, { value: "5", label: "5 秒" }, { value: "8", label: "8 秒（推荐）" },
    { value: "12", label: "12 秒" }, { value: "20", label: "20 秒" },
  ];
  const followUpChoices = $derived(followUpOptions.some((o) => o.value === String(followUp))
    ? followUpOptions : [...followUpOptions, { value: String(followUp), label: `${followUp} 秒` }]);
</script>

{#if bindings}
  <div class="space-y-6" data-testid="call-settings">
    <SettingsSection title={`呼叫${name}`} id="assistant-hotkey-title" description={`按一下就在右下角气泡里听你说，说完自动交给${name}；在听时再按一下立刻提交。`}>
      <SettingsRow label="呼叫键" description="默认是 ~ 键（Esc 下面那个）。清空 = 不用呼叫键，只用聊天页的按钮或唤醒词。" stacked>
        {#snippet control()}
          <HotkeyField id="assistant" title="呼叫键" value={bindings.hotkey} placeholder="如 ~ 或 F9" compact
            onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: companionBindingPatch(config, { hotkey }) })); }} />
        {/snippet}
      </SettingsRow>
      <SettingsRow label="回答完接着听" description={`${name}念完回答后接着听一会儿：直接接着说就行，不用再按呼叫键或喊唤醒词；没开口就安静结束，说“没事了”“拜拜”也会结束。她念的时候按呼叫键可以打断她。`}>
        {#snippet control()}
          <SelectField class="w-36" ariaLabel="回答完接着听" testid="call-follow-up" value={String(followUp)} options={followUpChoices}
            onchange={(v) => void patchSettings({ assistant: { followUpSeconds: Number(v) } }, Number(v) ? `回答完接着听 ${v} 秒` : "回答完不再接着听")} />
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
