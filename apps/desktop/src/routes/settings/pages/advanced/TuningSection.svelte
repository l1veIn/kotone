<script lang="ts">
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { TriangleAlert } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { m } from "$lib/paraglide/messages.js";
</script>

{#if $settingsStore}
  <div class="space-y-6">
  <p class="m-0 flex items-start gap-2 rounded-xl border border-warning/35 bg-warning/8 px-3.5 py-2.5 text-xs leading-relaxed text-warning">
    <TriangleAlert class="mt-0.5 size-3.5 shrink-0" />
    {m.system_tuning_warning()}
  </p>

  <SettingsSection title={m.system_tuning_vad_title()} description={m.system_tuning_vad_description()}>
    <SettingsRow label={m.system_tuning_vad_threshold()} stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label={m.system_tuning_vad_threshold()}
            min="0.1"
            max="0.9"
            step="0.05"
            value={$settingsStore.vad.threshold}
            onchange={(e) =>
              void patchSettings(
                { vad: { threshold: Number((e.target as HTMLInputElement).value) } },
                m.system_tuning_vad_threshold_saved(),
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.threshold.toFixed(2)}</span>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.system_tuning_min_speech()} stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label={m.system_tuning_min_speech()}
            min="20"
            max="500"
            step="10"
            value={$settingsStore.vad.minSpeechMs}
            onchange={(e) =>
              void patchSettings(
                { vad: { minSpeechMs: Number((e.target as HTMLInputElement).value) } },
                m.system_tuning_min_speech_saved(),
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.minSpeechMs} ms</span>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.system_tuning_min_silence()} stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label={m.system_tuning_min_silence()}
            min="20"
            max="500"
            step="10"
            value={$settingsStore.vad.minSilenceMs}
            onchange={(e) =>
              void patchSettings(
                { vad: { minSilenceMs: Number((e.target as HTMLInputElement).value) } },
                m.system_tuning_min_silence_saved(),
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.minSilenceMs} ms</span>
        </div>
      {/snippet}
    </SettingsRow>
    <p class="m-0 py-3 text-[11px] leading-relaxed text-muted-foreground">{m.system_tuning_vad_hint()}</p>
  </SettingsSection>

  <SettingsSection title={m.system_tuning_hotwords_title()} description={m.system_tuning_hotwords_description()}>
    <SettingsRow label={m.system_tuning_hotwords_score()} stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label={m.system_tuning_hotwords_score()}
            min="0"
            max="10"
            step="0.5"
            value={$settingsStore.hotwordsScore}
            onchange={(e) =>
              void patchSettings(
                { hotwordsScore: Number((e.target as HTMLInputElement).value) },
                m.system_tuning_hotwords_saved(),
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.hotwordsScore.toFixed(1)}</span>
        </div>
      {/snippet}
    </SettingsRow>
    <p class="m-0 py-3 text-[11px] leading-relaxed text-muted-foreground">{m.system_tuning_hotwords_hint()}</p>
  </SettingsSection>
  </div>
{/if}
