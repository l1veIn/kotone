<script lang="ts">
  // 「琴音 → 唤醒」（原「设置 → 怎么叫她」）：呼叫键、按下以后怎么听（说一句就走 / 录音笔 / 通话，ADR 033 2026-11-26 修订）；
  // 选了通话才有“通话多久挂断”和回声消除（ADR 033），下面接快捷指令等（ADR 028）。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
  import { type CallMode } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import HotkeyLink from "../../../../lib/components/HotkeyLink.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  const config = $derived($settingsStore?.assistant);
  const name = $derived($companionName);
  /** 通话多久没人说话挂断（新字段 assistant.callIdleSeconds，旧配置没有 = 30 秒；0 = 旧写法的“不通话”）。 */
  const callIdle = $derived(config?.callIdleSeconds ?? 30);
  /** 按下以后怎么听（新字段 assistant.callMode，旧配置没有 = 说一句就走）；选了通话可是 callIdleSeconds 是 0 = 说一句就走（同后端）。 */
  const callMode = $derived.by((): CallMode => {
    const mode = config?.callMode ?? "one-shot";
    return mode === "call" && callIdle === 0 ? "one-shot" : mode;
  });
  const modes: { id: CallMode; name: () => string; desc: () => string }[] = [
    { id: "one-shot", name: m.companion_call_mode_one_shot_name, desc: m.companion_call_mode_one_shot_desc },
    { id: "dictation", name: m.companion_call_mode_dictation_name, desc: m.companion_call_mode_dictation_desc },
    { id: "call", name: m.companion_call_mode_call_name, desc: m.companion_call_mode_call_desc },
  ];
  let savingMode = $state(false);
  async function selectMode(mode: (typeof modes)[number]) {
    if (savingMode || mode.id === callMode) return;
    savingMode = true;
    try {
      // 旧配置的“不通话”（callIdleSeconds 0）换成通话时用缺省的 30 秒
      const patch = mode.id === "call" && callIdle === 0 ? { callMode: mode.id, callIdleSeconds: 30 } : { callMode: mode.id };
      await patchSettings({ assistant: patch }, m.companion_call_mode_saved({ name: mode.name() }));
    } finally { savingMode = false; }
  }
  const callOptions = [
    { value: "15", label: m.companion_call_idle_seconds({ seconds: 15 }) },
    { value: "30", label: m.companion_call_idle_recommended() }, { value: "60", label: m.companion_call_idle_minutes({ minutes: 1 }) },
    { value: "180", label: m.companion_call_idle_minutes({ minutes: 3 }) }, { value: "600", label: m.companion_call_idle_minutes({ minutes: 10 }) },
  ];
  const callChoices = $derived(callOptions.some((o) => o.value === String(callIdle))
    ? callOptions : [...callOptions, { value: String(callIdle), label: m.companion_call_idle_seconds({ seconds: callIdle }) }]);
  const echoCancellation = $derived(config?.echoCancellation ?? true);
</script>

{#if config}
  <div class="space-y-6" data-testid="call-settings">
    <SettingsSection title={m.companion_call_title({ name })} id="assistant-hotkey-title" description={m.companion_call_desc({ name })}>
      <SettingsRow label={m.companion_call_hotkey_label()} description={m.companion_call_hotkey_desc()} stacked>
        {#snippet control()}
          <!-- 热键只在「设置 → 热键」改（ADR 040 D3） -->
          <HotkeyLink id="kotone.call" value={config.hotkey} />
        {/snippet}
      </SettingsRow>
      <div class="py-3.5">
        <p class="mt-0 mb-2 text-sm font-medium">{m.companion_call_mode_label()}</p>
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-3" role="group" aria-label={m.companion_call_mode_aria()}>
          {#each modes as mode (mode.id)}
            {@const active = callMode === mode.id}
            <button type="button" data-testid={`call-mode-${mode.id}`} aria-pressed={active} disabled={savingMode}
              class="rounded-xl border px-3 py-3 text-left transition-colors {active
                ? 'border-primary/60 bg-primary/10'
                : 'border-hairline bg-background/40 hover:bg-muted/60'}"
              onclick={() => void selectMode(mode)}>
              <p class="mt-0 mb-0 text-[13px] font-semibold {active ? 'text-primary' : 'text-foreground'}">{mode.name()}</p>
              <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{mode.desc()}</p>
            </button>
          {/each}
        </div>
      </div>
      {#if callMode === "call"}
        <SettingsRow label={m.companion_call_idle_label()} description={m.companion_call_idle_desc({ name })}>
          {#snippet control()}
            <SelectField class="w-44" ariaLabel={m.companion_call_idle_aria()} testid="call-idle" value={String(callIdle)} options={callChoices}
              onchange={(v) => void patchSettings({ assistant: { callIdleSeconds: Number(v) } }, m.companion_call_idle_saved({ seconds: v }))} />
          {/snippet}
        </SettingsRow>
        <ToggleField checked={echoCancellation} label={m.companion_call_echo_label()}
          desc={m.companion_call_echo_desc({ name })}
          testid="call-echo-cancellation"
          onchange={(on) => void patchSettings({ assistant: { echoCancellation: on } }, on ? m.companion_call_echo_on() : m.companion_call_echo_off())} />
      {/if}
    </SettingsSection>
    <CompanionSettings />
  </div>
{/if}
