<script lang="ts">
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { TriangleAlert } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
</script>

{#if $settingsStore}
  <div class="space-y-6">
  <p class="m-0 flex items-start gap-2 rounded-xl border border-warning/35 bg-warning/8 px-3.5 py-2.5 text-xs leading-relaxed text-warning">
    <TriangleAlert class="mt-0.5 size-3.5 shrink-0" />
    除非你知道你在做什么，否则不建议修改这些配置。默认值即原行为。
  </p>

  <SettingsSection title="VAD 高级设置" description="控制 silero 把背景噪声判成说话的程度，仅 one-shot / solo 生效（与通用页的「静音判停时长」是两回事）。下一条会话立即生效。">
    <SettingsRow label="语音判定阈值" stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label="语音判定阈值"
            min="0.1"
            max="0.9"
            step="0.05"
            value={$settingsStore.vad.threshold}
            onchange={(e) =>
              void patchSettings(
                { vad: { threshold: Number((e.target as HTMLInputElement).value) } },
                "VAD 阈值已保存",
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.threshold.toFixed(2)}</span>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="最短语音" stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label="最短语音"
            min="20"
            max="500"
            step="10"
            value={$settingsStore.vad.minSpeechMs}
            onchange={(e) =>
              void patchSettings(
                { vad: { minSpeechMs: Number((e.target as HTMLInputElement).value) } },
                "最短语音已保存",
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.minSpeechMs} ms</span>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="最短静音" stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label="最短静音"
            min="20"
            max="500"
            step="10"
            value={$settingsStore.vad.minSilenceMs}
            onchange={(e) =>
              void patchSettings(
                { vad: { minSilenceMs: Number((e.target as HTMLInputElement).value) } },
                "最短静音已保存",
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.vad.minSilenceMs} ms</span>
        </div>
      {/snippet}
    </SettingsRow>
    <p class="m-0 py-3 text-[11px] leading-relaxed text-muted-foreground">调高阈值、拉长最短语音可减少背景噪声被误判成语音。</p>
  </SettingsSection>

  <SettingsSection title="热词权重" description="热词命中加分：越高热词越容易命中，但背景噪声被识别成热词的概率也越高（如句首「DPS」）。">
    <SettingsRow label="热词加分" stacked>
      {#snippet control()}
        <div class="flex items-center gap-3">
          <input
            type="range"
            aria-label="热词加分"
            min="0"
            max="10"
            step="0.5"
            value={$settingsStore.hotwordsScore}
            onchange={(e) =>
              void patchSettings(
                { hotwordsScore: Number((e.target as HTMLInputElement).value) },
                "热词权重已保存",
              )}
            class="min-w-0 flex-1 accent-[var(--color-primary)]"
          />
          <span class="w-16 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{$settingsStore.hotwordsScore.toFixed(1)}</span>
        </div>
      {/snippet}
    </SettingsRow>
    <p class="m-0 py-3 text-[11px] leading-relaxed text-muted-foreground">0 = 无偏置（完全不鼓励热词）；改动需「停止 → 启动」后生效。</p>
  </SettingsSection>
  </div>
{/if}
