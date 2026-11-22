<script lang="ts">
  // 「设置 → 怎么叫她」：呼叫键（按一下就是和她通话，相当于按键版的语音唤醒）、通话多久没人说话挂断、
  // 回声消除（ADR 033），下面接快捷指令等（ADR 028）。
  import { companionName } from "$lib/stores/companion";
  import { companionBindingPatch, companionBindings, updateSettings } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  const config = $derived($settingsStore?.assistant);
  const bindings = $derived(companionBindings(config));
  const name = $derived($companionName);
  /** 通话多久没人说话挂断（新字段 assistant.callIdleSeconds，旧配置没有 = 30 秒；0 = 不通话）。 */
  const callIdle = $derived(config?.callIdleSeconds ?? 30);
  const callOptions = [
    { value: "0", label: "不通话（一问一答）" }, { value: "15", label: "15 秒" },
    { value: "30", label: "30 秒（推荐）" }, { value: "60", label: "1 分钟" }, { value: "180", label: "3 分钟" },
    { value: "600", label: "10 分钟" },
  ];
  const callChoices = $derived(callOptions.some((o) => o.value === String(callIdle))
    ? callOptions : [...callOptions, { value: String(callIdle), label: `${callIdle} 秒` }]);
  const echoCancellation = $derived(config?.echoCancellation ?? true);
</script>

{#if bindings}
  <div class="space-y-6" data-testid="call-settings">
    <SettingsSection title={`呼叫${name}`} id="assistant-hotkey-title" description={`按一下就是和${name}通话：一直在听，直接说就行，她说话时也能开口打断。说“退下”“安静”或再按一下就挂断。`}>
      <SettingsRow label="呼叫键" description="默认是 Ctrl + ~（~ 是 Esc 下面那个键）。清空 = 不用呼叫键，只用聊天页的按钮或唤醒词。" stacked>
        {#snippet control()}
          <HotkeyField id="assistant" title="呼叫键" value={bindings.hotkey} placeholder="如 ~ 或 F9" compact
            onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: companionBindingPatch(config, { hotkey }) })); }} />
        {/snippet}
      </SettingsRow>
      <SettingsRow label="通话" description={`多久没人说话就挂断（${name}在说、在做、在等你确认时不算）。选“不通话”就一问一答：按一下说一句，答完不再听。`}>
        {#snippet control()}
          <SelectField class="w-44" ariaLabel="通话多久没人说话挂断" testid="call-idle" value={String(callIdle)} options={callChoices}
            onchange={(v) => void patchSettings({ assistant: { callIdleSeconds: Number(v) } }, Number(v) ? `通话里 ${v} 秒没人说话就挂断` : "不通话：一问一答")} />
        {/snippet}
      </SettingsRow>
      <ToggleField checked={echoCancellation} label="开着音箱也能打断她"
        desc={`消掉音箱里放出来的声音（${name}的声音、游戏声），她说话时你直接开口就能打断。关掉以后她说话时不听，要打断就按呼叫键。戴耳机开着也没关系。`}
        testid="call-echo-cancellation"
        onchange={(on) => void patchSettings({ assistant: { echoCancellation: on } }, on ? "开着音箱也能打断她" : "她说话时不听")} />
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
