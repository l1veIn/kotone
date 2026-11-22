<script lang="ts">
  // 声音（ADR 028：原「音效」，麦克风也搬到这里——语音输入和呼叫共用一个麦克风）。
  import { onMount } from "svelte";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { listAudioDevices, previewSfx, setAudioDevice, type AudioDevice } from "../../../../lib/ipc";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { Play } from "@lucide/svelte";
  import { Switch } from "$lib/components/ui/switch";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { m } from "$lib/paraglide/messages.js";

  type EventKind = "record" | "send";

  let devices = $state<AudioDevice[]>([]);
  onMount(async () => {
    try { devices = await listAudioDevices(); } catch (error) { toast(false, m.system_sound_read_devices_failed({ error: errText(error) })); }
  });

  const recordSounds = [
    { value: "rise", label: m.system_sound_effect_rise() },
    { value: "ding-up", label: m.system_sound_effect_ding_up() },
    { value: "chirp-up", label: m.system_sound_effect_chirp_up() },
  ];
  const sendSounds = [
    { value: "fall", label: m.system_sound_effect_fall() },
    { value: "knock", label: m.system_sound_effect_knock() },
    { value: "ding-down", label: m.system_sound_effect_ding_down() },
  ];

  /** 切换音效：仅保存（试听走卡片上的播放键，避免切换时被打断/轰炸） */
  async function onPick(kind: EventKind, soundId: string) {
    await patchSettings(
      { soundFeedback: { [kind]: { soundId } } },
      kind === "record" ? m.system_sound_record_switched() : m.system_sound_send_switched(),
    );
  }

  const cards: {
    kind: EventKind;
    title: string;
    hint: string;
    options: { value: string; label: string }[];
  }[] = [
    { kind: "record", title: m.system_sound_record_title(), hint: m.system_sound_record_hint(), options: recordSounds },
    { kind: "send", title: m.system_sound_send_title(), hint: m.system_sound_send_hint(), options: sendSounds },
  ];
</script>

{#if $settingsStore}
  <div class="space-y-6">
  <SettingsSection title={m.system_sound_microphone()} description={m.system_sound_microphone_section_description()} testid="sound-microphone">
    <SettingsRow label={m.system_sound_microphone()} description={m.system_sound_microphone_row_description()}>
      {#snippet control()}
        <div class="w-72 max-w-full">
          <SelectField size="md" ariaLabel={m.system_sound_microphone()} value={$settingsStore.audioDeviceId}
            options={devices.map((device) => ({ value: device.id, label: device.name }))}
            onchange={async (id) => {
              try {
                await setAudioDevice(id);
                settingsStore.update((s) => (s ? { ...s, audioDeviceId: id } : s));
                toast(true, m.system_sound_microphone_switched());
              } catch (err) {
                toast(false, m.system_sound_microphone_switch_failed({ error: errText(err) }));
              }
            }} />
        </div>
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField checked={$settingsStore.lowerSpeakerVolumeWhileListening} label={m.system_sound_duck_label()}
        desc={m.system_sound_duck_description()}
        onchange={(enabled) => void patchSettings({ lowerSpeakerVolumeWhileListening: enabled }, m.system_sound_duck_saved())} />
    </div>
  </SettingsSection>

  <p class="m-0 px-1 text-[13px] leading-relaxed text-muted-foreground">
    {m.system_sound_intro()}
  </p>

  {#each cards as card}
    {@const item = $settingsStore.soundFeedback[card.kind]}
    <SettingsSection title={card.title} description={card.hint}>
      {#snippet actions()}
        <Switch
          aria-label={card.title}
          bind:checked={() => item.enabled, (value) =>
            void patchSettings(
              { soundFeedback: { [card.kind]: { enabled: value } } },
              value ? m.system_sound_card_enabled({ name: card.title }) : m.system_sound_card_disabled({ name: card.title }),
            )}
        />
      {/snippet}
      <SettingsRow label={m.system_sound_effect_label()} description={m.system_sound_effect_description()}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <SelectField
              class="w-40"
              ariaLabel={m.system_sound_card_effect_label({ name: card.title })}
              value={item.soundId}
              options={card.options}
              onchange={(soundId) => void onPick(card.kind, soundId)}
            />
            <button
              type="button"
              aria-label={m.system_sound_card_preview_label({ name: card.title })}
              class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-primary/35 bg-primary/10 text-primary transition-colors hover:bg-primary/20"
              onclick={() => void previewSfx(card.kind, item.soundId)}
            >
              <Play class="size-3.5 fill-current" />
            </button>
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.system_sound_card_volume_label({ name: card.title })} stacked>
        {#snippet control()}
          <div class="flex items-center gap-3">
            <input
              type="range"
              aria-label={m.system_sound_card_volume_label({ name: card.title })}
              min="10"
              max="100"
              step="5"
              value={item.volume}
              onchange={(e) =>
                void patchSettings(
                  { soundFeedback: { [card.kind]: { volume: Number((e.target as HTMLInputElement).value) } } },
                  m.system_sound_card_volume_saved({ name: card.title }),
                )}
              class="min-w-0 flex-1 accent-[var(--color-primary)]"
            />
            <span class="w-14 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{item.volume}%</span>
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>
  {/each}

  <SettingsSection title={m.system_sound_triggers_title()}>
    <ul class="m-0 list-none space-y-1.5 py-3.5 pl-0 text-xs leading-relaxed text-muted-foreground">
      <li>{m.system_sound_trigger_record()}</li>
      <li>{m.system_sound_trigger_send()}</li>
      <li>{m.system_sound_trigger_solo()}</li>
      <li>{m.system_sound_trigger_failure()}</li>
    </ul>
  </SettingsSection>
  <p class="m-0 px-1 text-[11px] leading-relaxed text-muted-foreground/80">
    {m.system_sound_custom_coming()}
  </p>
  </div>
{/if}
