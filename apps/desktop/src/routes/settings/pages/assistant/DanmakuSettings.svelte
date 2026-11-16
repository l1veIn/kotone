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
  <section class="space-y-4 py-4" data-testid="danmaku-settings" aria-labelledby="danmaku-settings-title">
    <div>
      <h3 id="danmaku-settings-title" class="m-0 text-[13px] font-medium text-foreground">弹幕</h3>
      <p class="mt-0.5 mb-0 text-xs text-muted-foreground">在游戏画面上方飘过的文字提示。</p>
    </div>

    <div class="grid gap-4 sm:grid-cols-2">
      <label class="block space-y-2 text-xs text-muted-foreground">
        <span class="flex justify-between"><span>速度</span><span class="text-foreground/85">{config.danmakuSpeedPxPerSec} px/s</span></span>
        <input aria-label="弹幕速度" type="range" min="40" max="600" step="10" value={config.danmakuSpeedPxPerSec} disabled={saving}
          onchange={(event) => void save({ danmakuSpeedPxPerSec: event.currentTarget.valueAsNumber })} class="w-full accent-primary" />
        <span class="flex justify-between text-[10px] text-muted-foreground/80"><span>慢一些</span><span>快一些</span></span>
      </label>
      <label class="block space-y-2 text-xs text-muted-foreground">
        <span class="flex justify-between"><span>字号</span><span class="text-foreground/85">{config.danmakuFontSize} px</span></span>
        <input aria-label="弹幕字号" type="range" min="16" max="72" step="1" value={config.danmakuFontSize} disabled={saving}
          onchange={(event) => void save({ danmakuFontSize: event.currentTarget.valueAsNumber })} class="w-full accent-primary" />
        <span class="flex justify-between text-[10px] text-muted-foreground/80"><span>小</span><span>大</span></span>
      </label>
    </div>

    <div class="space-y-2 text-xs text-muted-foreground">
      <div class="flex items-center justify-between gap-3">
        <label for="danmaku-top-offset">距顶部</label>
        <label class="flex items-center gap-2 text-[11px]">
          <input type="number" aria-label="弹幕距顶部像素" min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
            aria-invalid={!!topError} aria-describedby={topError ? "danmaku-top-error" : "danmaku-top-help"}
            onchange={(event) => saveTop(event.currentTarget.valueAsNumber)}
            onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); saveTop(event.currentTarget.valueAsNumber); } }}
            class="h-7 w-20 rounded-md border border-input bg-background/60 px-2 text-right text-xs text-foreground outline-none focus:border-ring aria-invalid:border-destructive" />
          <span>px</span>
        </label>
      </div>
      <input id="danmaku-top-offset" aria-label="弹幕距顶部" type="range" min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
        onchange={(event) => saveTop(event.currentTarget.valueAsNumber)} class="w-full accent-primary" />
      <p id="danmaku-top-help" class="m-0 text-[10px] leading-relaxed text-muted-foreground/80">超出屏幕时自动调整。</p>
      {#if topError}<p id="danmaku-top-error" role="alert" class="m-0 text-[11px] text-destructive">{topError}</p>{/if}
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <span class="mr-2 text-xs text-muted-foreground">文字颜色</span>
      {#each colors as color}
        <button type="button" aria-label={`弹幕颜色${color.label}`} aria-pressed={config.danmakuColor.toUpperCase() === color.value}
          class="choice size-7 rounded-full p-1" style:--swatch={color.value} disabled={saving} onclick={() => void save({ danmakuColor: color.value })}>
          <span class="block size-full rounded-full ring-1 ring-black/20" style:background="var(--swatch)" aria-hidden="true"></span>
        </button>
      {/each}
      <label class="ml-1 flex items-center gap-2 text-[11px] text-muted-foreground">
        <input type="color" aria-label="自定义弹幕颜色" value={config.danmakuColor} disabled={saving}
          onchange={(event) => void save({ danmakuColor: event.currentTarget.value.toUpperCase() })} class="h-7 w-9 cursor-pointer rounded border border-hairline bg-transparent" />
        <span class="font-mono">{config.danmakuColor.toUpperCase()}</span>
      </label>
    </div>

    <fieldset disabled={saving} class="m-0 border-0 p-0">
      <legend class="mb-2 text-xs text-muted-foreground">表情</legend>
      <div class="flex flex-wrap gap-2">
        {#each DANMAKU_EMOTES as emote}
          <button type="button" aria-label={`弹幕表情${emote.label}`} aria-pressed={config.danmakuEmote === emote.id}
            class="choice flex h-16 w-16 flex-col items-center justify-center gap-0.5 rounded-xl text-[10px] text-muted-foreground" onclick={() => void save({ danmakuEmote: emote.id })}>
            {#if emote.src}<img src={emote.src} alt="" width="34" height="34" class="object-contain" />{:else}<span class="text-lg" aria-hidden="true">Aa</span>{/if}
            <span>{emote.label}</span>
          </button>
        {/each}
      </div>
    </fieldset>

    <div class="overflow-hidden rounded-xl border border-hairline bg-[#11111c] px-4 py-3">
      <p class="mt-0 mb-2 text-[10px] text-white/50">预览</p>
      <div class="sample" style:color={preview.color} style:font-size={`${Math.min(preview.fontSize, 36)}px`} data-testid="danmaku-style-preview">
        {#if preview.emote.src}<img src={preview.emote.src} alt="" />{/if}
        <span>{sample}</span>
      </div>
    </div>
  </section>
{/if}

<style>
  .choice { border: 1px solid var(--color-hairline); cursor: pointer; transition: border-color 120ms, background-color 120ms; }
  .choice:hover { background-color: color-mix(in oklab, var(--color-muted) 60%, transparent); }
  .choice[aria-pressed="true"] { border-color: var(--color-primary); background-color: color-mix(in oklab, var(--color-primary) 10%, transparent); color: var(--color-foreground); }
  button:disabled { cursor: wait; opacity: .5; }
  button:focus-visible { outline: 2px solid var(--color-ring); outline-offset: 3px; }
  .sample { display: flex; align-items: center; gap: .2em; white-space: nowrap; font-weight: 700; line-height: 1.35; text-shadow: 1px 1px 2px #000, -1px -1px 2px #000; }
  .sample img { width: 1.4em; height: 1.4em; object-fit: contain; }
</style>
