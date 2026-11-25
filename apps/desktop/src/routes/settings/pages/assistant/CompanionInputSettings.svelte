<script lang="ts">
  // 「琴音 → 唤醒」（原「设置 → 怎么叫她」）：呼叫键（按一下就是和她通话，相当于按键版的语音唤醒）、通话多久没人说话挂断、
  // 回声消除（ADR 033），下面接快捷指令等（ADR 028）。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
  import { companionBindings } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import HotkeyLink from "../../../../lib/components/HotkeyLink.svelte";
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
    { value: "0", label: m.companion_call_idle_off() }, { value: "15", label: m.companion_call_idle_seconds({ seconds: 15 }) },
    { value: "30", label: m.companion_call_idle_recommended() }, { value: "60", label: m.companion_call_idle_minutes({ minutes: 1 }) },
    { value: "180", label: m.companion_call_idle_minutes({ minutes: 3 }) }, { value: "600", label: m.companion_call_idle_minutes({ minutes: 10 }) },
  ];
  const callChoices = $derived(callOptions.some((o) => o.value === String(callIdle))
    ? callOptions : [...callOptions, { value: String(callIdle), label: m.companion_call_idle_seconds({ seconds: callIdle }) }]);
  const echoCancellation = $derived(config?.echoCancellation ?? true);
</script>

{#if bindings}
  <div class="space-y-6" data-testid="call-settings">
    <SettingsSection title={m.companion_call_title({ name })} id="assistant-hotkey-title" description={m.companion_call_desc({ name })}>
      <SettingsRow label={m.companion_call_hotkey_label()} description={m.companion_call_hotkey_desc()} stacked>
        {#snippet control()}
          <!-- 热键只在「设置 → 热键」改（ADR 040 D3） -->
          <HotkeyLink id="kotone.call" value={bindings.hotkey} />
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.companion_call_idle_label()} description={m.companion_call_idle_desc({ name })}>
        {#snippet control()}
          <SelectField class="w-44" ariaLabel={m.companion_call_idle_aria()} testid="call-idle" value={String(callIdle)} options={callChoices}
            onchange={(v) => void patchSettings({ assistant: { callIdleSeconds: Number(v) } }, Number(v) ? m.companion_call_idle_saved({ seconds: v }) : m.companion_call_idle_off_saved())} />
        {/snippet}
      </SettingsRow>
      <ToggleField checked={echoCancellation} label={m.companion_call_echo_label()}
        desc={m.companion_call_echo_desc({ name })}
        testid="call-echo-cancellation"
        onchange={(on) => void patchSettings({ assistant: { echoCancellation: on } }, on ? m.companion_call_echo_on() : m.companion_call_echo_off())} />
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
