<script lang="ts">
  import { DANMAKU_EMOTES, danmakuStyle, type DanmakuEmote } from "../../../../lib/danmaku";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import type { AssistantConfig } from "../../../../lib/ipc";

  type Appearance = Pick<AssistantConfig, "danmakuSpeedPxPerSec" | "danmakuFontSize" | "danmakuTopOffsetPx" | "danmakuColor" | "danmakuEmote">;
  let saving = $state(false);
  let topError = $state("");
  const config = $derived($settingsStore?.assistant);
  const sample = "该打小龙了";
  const preview = $derived(danmakuStyle({
    text: sample,
    fontSize: config?.danmakuFontSize ?? 32,
    color: config?.danmakuColor ?? "#FFFFFF",
    emote: (config?.danmakuEmote ?? "none") as DanmakuEmote,
  }));
  const colors = [
    { value: "#FFFFFF", label: "白色" },
    { value: "#00E5FF", label: "霓虹青" },
    { value: "#FF6DA8", label: "樱粉" },
    { value: "#FFE082", label: "暖黄" },
  ];

  async function save(patch: Partial<Appearance>) {
    if (saving) return false;
    saving = true;
    try {
      return await patchSettings({ assistant: patch }, "弹幕样式已保存");
    } finally {
      saving = false;
    }
  }
  async function saveTop(value: number) {
    if (saving) return;
    if (!Number.isInteger(value) || value < 0 || value > 2160) {
      topError = "请输入 0 到 2160 之间的整数像素。";
      return;
    }
    topError = "";
    if (value !== config?.danmakuTopOffsetPx && !await save({ danmakuTopOffsetPx: value })) {
      topError = "距顶部未保存，请重新输入或按 Enter 重试。";
    }
  }
</script>

{#if config}
  <section class="kotone-panel space-y-4 p-4" data-testid="danmaku-settings" aria-labelledby="danmaku-settings-title">
    <div>
      <h2 id="danmaku-settings-title" class="text-sm font-semibold text-kotone-cyan/90">弹幕</h2>
    </div>

    <div class="grid gap-4 sm:grid-cols-2">
      <label class="space-y-2 text-xs text-white/70">
        <span class="flex justify-between"><span>速度</span><span>{config.danmakuSpeedPxPerSec} px/s</span></span>
        <input aria-label="弹幕速度" type="range" min="40" max="600" step="10" value={config.danmakuSpeedPxPerSec} disabled={saving}
          onchange={(event) => void save({ danmakuSpeedPxPerSec: event.currentTarget.valueAsNumber })} class="w-full accent-kotone-cyan" />
        <span class="flex justify-between text-[10px] text-white/35"><span>慢一些</span><span>快一些</span></span>
      </label>
      <label class="space-y-2 text-xs text-white/70">
        <span class="flex justify-between"><span>字号</span><span>{config.danmakuFontSize} px</span></span>
        <input aria-label="弹幕字号" type="range" min="16" max="72" step="1" value={config.danmakuFontSize} disabled={saving}
          onchange={(event) => void save({ danmakuFontSize: event.currentTarget.valueAsNumber })} class="w-full accent-kotone-cyan" />
        <span class="flex justify-between text-[10px] text-white/35"><span>小</span><span>大</span></span>
      </label>
    </div>

    <div class="space-y-2 text-xs text-white/70">
      <div class="flex items-center justify-between gap-3">
        <label for="danmaku-top-offset">距顶部</label>
        <label class="flex items-center gap-2 text-[11px] text-white/50">
          <input type="number" aria-label="弹幕距顶部像素" min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
            aria-invalid={!!topError} aria-describedby={topError ? "danmaku-top-error" : "danmaku-top-help"}
            onchange={(event) => saveTop(event.currentTarget.valueAsNumber)}
            onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); saveTop(event.currentTarget.valueAsNumber); } }}
            class="w-20 rounded-md border border-white/15 bg-black/15 px-2 py-1 text-right text-xs text-white/85 outline-none focus:border-kotone-cyan/60" />
          <span>px</span>
        </label>
      </div>
      <input id="danmaku-top-offset" aria-label="弹幕距顶部" type="range" min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
        onchange={(event) => saveTop(event.currentTarget.valueAsNumber)} class="w-full accent-kotone-cyan" />
      <p id="danmaku-top-help" class="text-[10px] leading-relaxed text-white/35">超出屏幕时自动调整。</p>
      {#if topError}<p id="danmaku-top-error" role="alert" class="text-[11px] text-kotone-pink">{topError}</p>{/if}
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <span class="mr-2 text-xs text-white/70">文字颜色</span>
      {#each colors as color}
        <button type="button" aria-label={`弹幕颜色${color.label}`} aria-pressed={config.danmakuColor.toUpperCase() === color.value}
          class="color-choice" style:--swatch={color.value} disabled={saving} onclick={() => void save({ danmakuColor: color.value })}>
          <span aria-hidden="true"></span>
        </button>
      {/each}
      <label class="ml-1 flex items-center gap-2 text-[11px] text-white/50">
        <input type="color" aria-label="自定义弹幕颜色" value={config.danmakuColor} disabled={saving}
          onchange={(event) => void save({ danmakuColor: event.currentTarget.value.toUpperCase() })} class="h-7 w-9 cursor-pointer rounded border border-white/15 bg-transparent" />
        <span>{config.danmakuColor.toUpperCase()}</span>
      </label>
    </div>

    <fieldset disabled={saving}>
      <legend class="mb-2 text-xs text-white/70">表情</legend>
      <div class="flex flex-wrap gap-2">
        {#each DANMAKU_EMOTES as emote}
          <button type="button" aria-label={`弹幕表情${emote.label}`} aria-pressed={config.danmakuEmote === emote.id}
            class="emote-choice" onclick={() => void save({ danmakuEmote: emote.id })}>
            {#if emote.src}<img src={emote.src} alt="" width="34" height="34" />{:else}<span class="text-lg" aria-hidden="true">Aa</span>{/if}
            <span>{emote.label}</span>
          </button>
        {/each}
      </div>
    </fieldset>

    <div class="overflow-hidden border-t border-white/8 pt-3">
      <p class="mb-2 text-[10px] text-white/35">预览</p>
      <div class="sample" style:color={preview.color} style:font-size={`${Math.min(preview.fontSize, 36)}px`} data-testid="danmaku-style-preview">
        {#if preview.emote.src}<img src={preview.emote.src} alt="" />{/if}
        <span>{sample}</span>
      </div>
    </div>
  </section>
{/if}

<style>
  .color-choice { width: 27px; height: 27px; padding: 4px; border: 1px solid #ffffff24; border-radius: 50%; cursor: pointer; }
  .color-choice span { display: block; width: 100%; height: 100%; border-radius: 50%; background: var(--swatch); }
  .color-choice[aria-pressed="true"], .emote-choice[aria-pressed="true"] { border-color: #00e5ff; background-color: #00e5ff12; }
  .emote-choice { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; width: 65px; height: 64px; border: 1px solid #ffffff15; border-radius: 9px; color: #ffffffa0; font-size: 10px; cursor: pointer; }
  .emote-choice img { object-fit: contain; }
  button:disabled { cursor: wait; opacity: .5; }
  button:focus-visible { outline: 2px solid #00e5ff; outline-offset: 3px; }
  .sample { display: flex; align-items: center; gap: .2em; white-space: nowrap; font-weight: 700; line-height: 1.35; text-shadow: 1px 1px 2px #000, -1px -1px 2px #000; }
  .sample img { width: 1.4em; height: 1.4em; object-fit: contain; }
</style>
