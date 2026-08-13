<script lang="ts">
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
</script>

{#if $settingsStore}
  <div class="flex items-start gap-2 rounded-lg bg-yellow-400/10 px-3 py-2 ring-1 ring-yellow-400/30">
    <span class="mt-0.5 text-xs text-yellow-300">⚠</span>
    <p class="text-[11px] leading-relaxed text-yellow-200/90">
      除非你知道你在做什么，否则不建议修改这些配置。默认值即原行为。
    </p>
  </div>

  <section class="kotone-panel mt-3 flex flex-col gap-4 p-4">
    <div>
      <h2 class="text-sm font-semibold text-kotone-cyan/90">VAD 高级设置</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">
        控制 silero 把背景噪声判成说话的程度，仅 one-shot / solo 生效（与通用页的「静音判停时长」是两回事）。下一条会话立即生效。
      </p>
    </div>
    <label class="block">
      <div class="flex items-center justify-between">
        <span class="text-xs text-white/70">语音判定阈值</span>
        <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
          {$settingsStore.vad.threshold.toFixed(2)}
        </span>
      </div>
      <input
        type="range"
        min="0.1"
        max="0.9"
        step="0.05"
        value={$settingsStore.vad.threshold}
        onchange={(e) =>
          void patchSettings(
            { vad: { threshold: Number((e.target as HTMLInputElement).value) } },
            "VAD 阈值已保存",
          )}
        class="mt-2 w-full accent-kotone-cyan"
      />
    </label>
    <label class="block">
      <div class="flex items-center justify-between">
        <span class="text-xs text-white/70">最短语音</span>
        <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
          {$settingsStore.vad.minSpeechMs} ms
        </span>
      </div>
      <input
        type="range"
        min="20"
        max="500"
        step="10"
        value={$settingsStore.vad.minSpeechMs}
        onchange={(e) =>
          void patchSettings(
            { vad: { minSpeechMs: Number((e.target as HTMLInputElement).value) } },
            "最短语音已保存",
          )}
        class="mt-2 w-full accent-kotone-cyan"
      />
    </label>
    <label class="block">
      <div class="flex items-center justify-between">
        <span class="text-xs text-white/70">最短静音</span>
        <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
          {$settingsStore.vad.minSilenceMs} ms
        </span>
      </div>
      <input
        type="range"
        min="20"
        max="500"
        step="10"
        value={$settingsStore.vad.minSilenceMs}
        onchange={(e) =>
          void patchSettings(
            { vad: { minSilenceMs: Number((e.target as HTMLInputElement).value) } },
            "最短静音已保存",
          )}
        class="mt-2 w-full accent-kotone-cyan"
      />
    </label>
    <p class="text-[10px] leading-relaxed text-white/35">
      调高阈值、拉长最短语音可减少背景噪声被误判成语音。
    </p>
  </section>

  <section class="kotone-panel mt-3 flex flex-col gap-4 p-4">
    <div>
      <h2 class="text-sm font-semibold text-kotone-cyan/90">热词权重</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">
        热词命中加分：越高热词越容易命中，但背景噪声被识别成热词的概率也越高（如句首「DPS」）。
      </p>
    </div>
    <label class="block">
      <div class="flex items-center justify-between">
        <span class="text-xs text-white/70">热词加分</span>
        <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
          {$settingsStore.hotwordsScore.toFixed(1)}
        </span>
      </div>
      <input
        type="range"
        min="0"
        max="10"
        step="0.5"
        value={$settingsStore.hotwordsScore}
        onchange={(e) =>
          void patchSettings(
            { hotwordsScore: Number((e.target as HTMLInputElement).value) },
            "热词权重已保存",
          )}
        class="mt-2 w-full accent-kotone-cyan"
      />
    </label>
    <p class="text-[10px] leading-relaxed text-white/35">
      0 = 无偏置（完全不鼓励热词）；改动需「停止 → 启动」后生效。
    </p>
  </section>
{/if}
