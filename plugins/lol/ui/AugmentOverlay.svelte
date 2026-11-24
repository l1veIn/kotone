<script lang="ts">
  // 覆盖层：海斗三选一时贴在每张海克斯卡片上面的分（照 aramhex）。比这个英雄平均胜率高 / 低几个百分点、胜率、局数；
  // 三张里最好的那张标出来，样本少的变淡；小字写数据从哪来（aramhex-data，要署名）。
  // 只读、不能点、没有动画；字号跟着宿主给的区域大小走（cqh），放不下就截掉。
  import type { PluginOverlayProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { AugmentOverlayData } from "./types";
  import { compact, rate, signed } from "./format";

  let { data }: PluginOverlayProps = $props();
  const card = $derived(data as AugmentOverlayData | null);
</script>

{#if card}
  {@const s = card.score}
  <div class="chip" class:best={card.best} class:low={s?.lowSample} data-testid="lol-overlay-augment"
    data-best={card.best ? "true" : "false"} data-augment={card.augment}>
    <div class="main">
      {#if s}
        <span class="delta" class:up={s.delta >= 0} class:down={s.delta < 0}>{signed(s.delta)}</span>
        <span class="stat">{rate(s.winRate)} · {m.plugin_lol_overlay_games({ games: compact(s.games) })}</span>
      {:else}
        <span class="stat">{m.plugin_lol_overlay_no_data()}</span>
      {/if}
    </div>
    <div class="foot">
      {#if card.best}<span class="tag">{m.plugin_lol_overlay_best()}</span>{/if}
      {#if s?.lowSample}<span class="tag low-tag">{m.plugin_lol_low_sample()}</span>{/if}
      <span class="credit">{card.attribution}</span>
    </div>
  </div>
{/if}

<style>
  .chip {
    box-sizing: border-box; width: 100%; height: 100%; overflow: hidden; padding: 0 4cqw;
    display: flex; flex-direction: column; justify-content: center; gap: 2cqh;
    background: rgb(10 12 18 / 0.84); border: 1px solid rgb(255 255 255 / 0.16); border-radius: 6px;
    color: #eef1f6; font-size: clamp(10px, 38cqh, 20px); line-height: 1.15; text-shadow: 0 1px 1px #000;
  }
  .chip.best { border-color: #f5c451; box-shadow: inset 0 0 0 1px #f5c451; }
  .chip.low { opacity: 0.7; }
  .main { display: flex; align-items: baseline; gap: 0.45em; white-space: nowrap; overflow: hidden; }
  .delta { font-weight: 700; font-variant-numeric: tabular-nums; }
  .delta.up { color: #6ee7a0; }
  .delta.down { color: #fca5a5; }
  .stat { color: #d6dde6; font-size: 0.85em; font-variant-numeric: tabular-nums; overflow: hidden; text-overflow: ellipsis; }
  .foot { display: flex; align-items: center; gap: 0.5em; font-size: 0.55em; white-space: nowrap; overflow: hidden; }
  .tag { padding: 0 0.35em; border-radius: 3px; background: rgb(245 196 81 / 0.22); color: #f5c451; font-weight: 600; }
  .low-tag { background: rgb(255 255 255 / 0.1); color: #cfd8e3; font-weight: 400; }
  .credit { margin-left: auto; color: #7b8794; overflow: hidden; text-overflow: ellipsis; }
</style>
