<script lang="ts">
  import { DANMAKU_EMOTES, danmakuStyle, type DanmakuEmote } from "../../../../lib/danmaku";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import type { AssistantConfig } from "../../../../lib/ipc";
  import relayRoom from "../../../../assets/brand/relay-room-bg.webp";

  /** 预览是一块缩小的 1920 宽屏幕：字号、距顶部、速度都按同一个比例缩小，看起来和游戏里一样。 */
  const SCREEN_WIDTH = 1920;
  let screenWidth = $state(0);
  const scale = $derived(screenWidth > 0 ? screenWidth / SCREEN_WIDTH : 0);

  /** 让示例弹幕在预览屏幕里从右飘到左（循环），速度按比例缩小。 */
  function scroll(node: HTMLElement, speed: number) {
    let animation: Animation | undefined;
    const start = (pxPerSec: number) => {
      animation?.cancel();
      const box = node.parentElement?.getBoundingClientRect().width ?? 0;
      const width = node.getBoundingClientRect().width;
      if (!box || !pxPerSec || matchMedia("(prefers-reduced-motion: reduce)").matches) {
        node.style.transform = `translate3d(${Math.max(0, (box - width) / 2)}px,0,0)`;
        return;
      }
      animation = node.animate(
        [{ transform: `translate3d(${box}px,0,0)` }, { transform: `translate3d(${-width}px,0,0)` }],
        { duration: ((box + width) / pxPerSec) * 1000, iterations: Infinity, easing: "linear" },
      );
    };
    start(speed);
    return { update: start, destroy: () => animation?.cancel() };
  }

  type Appearance = Pick<AssistantConfig, "danmakuSpeedPxPerSec" | "danmakuFontSize" | "danmakuTopOffsetPx" | "danmakuColor" | "danmakuEmote">;
  let saving = $state(false);
  let topError = $state("");
  const config = $derived($settingsStore?.assistant);
  const sample = m.companion_danmaku_sample();
  const preview = $derived(danmakuStyle({
    text: sample,
    fontSize: config?.danmakuFontSize ?? 32,
    color: config?.danmakuColor ?? "#FFFFFF",
    emote: (config?.danmakuEmote ?? "none") as DanmakuEmote,
  }));
  const colors = [
    { value: "#FFFFFF", label: m.companion_danmaku_color_white() },
    { value: "#00E5FF", label: m.companion_danmaku_color_cyan() },
    { value: "#FF6DA8", label: m.companion_danmaku_color_pink() },
    { value: "#FFE082", label: m.companion_danmaku_color_yellow() },
  ];

  async function save(patch: Partial<Appearance>) {
    if (saving) return false;
    saving = true;
    try {
      return await patchSettings({ assistant: patch }, m.companion_danmaku_saved());
    } finally {
      saving = false;
    }
  }
  async function saveTop(value: number) {
    if (saving) return;
    if (!Number.isInteger(value) || value < 0 || value > 2160) {
      topError = m.companion_danmaku_top_invalid();
      return;
    }
    topError = "";
    if (value !== config?.danmakuTopOffsetPx && !await save({ danmakuTopOffsetPx: value })) {
      topError = m.companion_danmaku_top_not_saved();
    }
  }
</script>

{#if config}
  <section class="space-y-4 py-4" data-testid="danmaku-settings" aria-labelledby="danmaku-settings-title">
    <div>
      <h3 id="danmaku-settings-title" class="m-0 text-[13px] font-medium text-foreground">{m.companion_danmaku_title()}</h3>
      <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.companion_danmaku_desc()}</p>
    </div>

    <div class="grid gap-4 sm:grid-cols-2">
      <label class="block space-y-2 text-xs text-muted-foreground">
        <span class="flex justify-between"><span>{m.companion_danmaku_speed()}</span><span class="text-foreground/85">{config.danmakuSpeedPxPerSec} px/s</span></span>
        <input aria-label={m.companion_danmaku_speed_aria()} type="range" min="40" max="600" step="10" value={config.danmakuSpeedPxPerSec} disabled={saving}
          onchange={(event) => void save({ danmakuSpeedPxPerSec: event.currentTarget.valueAsNumber })} class="w-full accent-primary" />
        <span class="flex justify-between text-[10px] text-muted-foreground/80"><span>{m.companion_danmaku_speed_slow()}</span><span>{m.companion_danmaku_speed_fast()}</span></span>
      </label>
      <label class="block space-y-2 text-xs text-muted-foreground">
        <span class="flex justify-between"><span>{m.companion_danmaku_font_size()}</span><span class="text-foreground/85">{config.danmakuFontSize} px</span></span>
        <input aria-label={m.companion_danmaku_font_size_aria()} type="range" min="16" max="72" step="1" value={config.danmakuFontSize} disabled={saving}
          onchange={(event) => void save({ danmakuFontSize: event.currentTarget.valueAsNumber })} class="w-full accent-primary" />
        <span class="flex justify-between text-[10px] text-muted-foreground/80"><span>{m.companion_danmaku_font_size_small()}</span><span>{m.companion_danmaku_font_size_large()}</span></span>
      </label>
    </div>

    <div class="space-y-2 text-xs text-muted-foreground">
      <div class="flex items-center justify-between gap-3">
        <label for="danmaku-top-offset">{m.companion_danmaku_top()}</label>
        <label class="flex items-center gap-2 text-[11px]">
          <input type="number" aria-label={m.companion_danmaku_top_px_aria()} min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
            aria-invalid={!!topError} aria-describedby={topError ? "danmaku-top-error" : "danmaku-top-help"}
            onchange={(event) => saveTop(event.currentTarget.valueAsNumber)}
            onkeydown={(event) => { if (event.key === "Enter") { event.preventDefault(); saveTop(event.currentTarget.valueAsNumber); } }}
            class="h-7 w-20 rounded-md border border-input bg-background/60 px-2 text-right text-xs text-foreground outline-none focus:border-ring aria-invalid:border-destructive" />
          <span>px</span>
        </label>
      </div>
      <input id="danmaku-top-offset" aria-label={m.companion_danmaku_top_aria()} type="range" min="0" max="2160" step="1" value={config.danmakuTopOffsetPx ?? 120} disabled={saving}
        onchange={(event) => saveTop(event.currentTarget.valueAsNumber)} class="w-full accent-primary" />
      <p id="danmaku-top-help" class="m-0 text-[10px] leading-relaxed text-muted-foreground/80">{m.companion_danmaku_top_help()}</p>
      {#if topError}<p id="danmaku-top-error" role="alert" class="m-0 text-[11px] text-destructive">{topError}</p>{/if}
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <span class="mr-2 text-xs text-muted-foreground">{m.companion_danmaku_color()}</span>
      {#each colors as color}
        <button type="button" aria-label={m.companion_danmaku_color_aria({ color: color.label })} aria-pressed={config.danmakuColor.toUpperCase() === color.value}
          class="choice size-7 rounded-full p-1" style:--swatch={color.value} disabled={saving} onclick={() => void save({ danmakuColor: color.value })}>
          <span class="block size-full rounded-full ring-1 ring-black/20" style:background="var(--swatch)" aria-hidden="true"></span>
        </button>
      {/each}
      <label class="ml-1 flex items-center gap-2 text-[11px] text-muted-foreground">
        <input type="color" aria-label={m.companion_danmaku_color_custom_aria()} value={config.danmakuColor} disabled={saving}
          onchange={(event) => void save({ danmakuColor: event.currentTarget.value.toUpperCase() })} class="h-7 w-9 cursor-pointer rounded border border-hairline bg-transparent" />
        <span class="font-mono">{config.danmakuColor.toUpperCase()}</span>
      </label>
    </div>

    <fieldset disabled={saving} class="m-0 border-0 p-0">
      <legend class="mb-2 text-xs text-muted-foreground">{m.companion_danmaku_emote()}</legend>
      <div class="flex flex-wrap gap-2">
        {#each DANMAKU_EMOTES as emote}
          <button type="button" aria-label={m.companion_danmaku_emote_aria({ emote: emote.label })} aria-pressed={config.danmakuEmote === emote.id}
            class="choice flex h-16 w-16 flex-col items-center justify-center gap-0.5 rounded-xl text-[10px] text-muted-foreground" onclick={() => void save({ danmakuEmote: emote.id })}>
            {#if emote.src}<img src={emote.src} alt="" width="34" height="34" class="object-contain" />{:else}<span class="text-lg" aria-hidden="true">Aa</span>{/if}
            <span>{emote.label}</span>
          </button>
        {/each}
      </div>
    </fieldset>

    <div>
      <p class="mt-0 mb-1.5 text-xs text-muted-foreground">{m.companion_danmaku_preview()}</p>
      <div class="screen" bind:clientWidth={screenWidth} style:background-image={`url(${relayRoom})`} data-testid="danmaku-screen">
        {#if scale > 0}
          {#key `${preview.fontSize}|${preview.emote.id}|${preview.color}`}
            <div class="sample" use:scroll={(config.danmakuSpeedPxPerSec ?? 160) * scale}
              style:top={`${Math.max(0, Math.min(config.danmakuTopOffsetPx * scale, screenWidth * 9 / 16 - preview.fontSize * scale * 1.4))}px`}
              style:color={preview.color} style:font-size={`${Math.max(8, preview.fontSize * scale)}px`} data-testid="danmaku-style-preview">
              {#if preview.emote.src}<img src={preview.emote.src} alt="" />{/if}
              <span>{sample}</span>
            </div>
          {/key}
        {/if}
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
  .screen { position: relative; width: 100%; max-width: 30rem; aspect-ratio: 16 / 9; overflow: hidden; border-radius: .75rem; border: 1px solid var(--color-hairline); background-color: #11111c; background-size: cover; background-position: center; }
  .screen::after { content: ""; position: absolute; inset: 0; background: linear-gradient(#0006, #0003); pointer-events: none; }
  .sample { position: absolute; left: 0; z-index: 1; display: inline-flex; align-items: center; gap: .2em; width: max-content; white-space: nowrap; font-weight: 700; line-height: 1.35; text-shadow: 1px 1px 2px #000, -1px -1px 2px #000, 0 2px 4px #0009; will-change: transform; }
  .sample img { width: 1.4em; height: 1.4em; object-fit: contain; }
</style>
