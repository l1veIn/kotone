<script lang="ts">
  // 覆盖层：海斗三选一时贴在每张海克斯卡片上面的标签（照 AramHex 的三选一标签，MIT；规则在 plugins/lol/src/overlays/rules.rs）。
  // 第一行：评级小牌（同稀有度里的名次换算 S–D）+ 判断（首选 胜色 / 可选 正文色 / 可考虑刷新 负色，配字）；
  // 第二行：比这个英雄平均胜率高 / 低几个百分点、胜率、局数；第三行：依据（“金色第 3/40 · 样本少”）和出处（aramhex-data，要署名）。
  // 首选那张用 2 px 白色墨线圈出来（设计文档第四节 4：不用金色），不到 300 局的变淡。
  // 只读、不能点、没有动画；字号跟着宿主给的区域高度走（cqh），放不下就截掉。
  import "./overlay.css";
  import type { PluginOverlayProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { AugmentOverlayData } from "./types";
  import { compact, rate, signed } from "./format";

  let { data }: PluginOverlayProps = $props();
  const card = $derived(data as AugmentOverlayData | null);

  function verdictText(c: AugmentOverlayData): string {
    if (c.verdict === "best") return c.sampleWarning ? m.plugin_lol_overlay_verdict_best_thin() : m.plugin_lol_overlay_verdict_best();
    if (c.thin === "tooFew") return m.plugin_lol_overlay_verdict_too_few();
    if (c.verdict === "reroll") return m.plugin_lol_overlay_verdict_reroll();
    if (c.verdict === "ok") return m.plugin_lol_overlay_verdict_ok();
    return m.plugin_lol_overlay_verdict_no_data();
  }

  const RARITY: Record<string, () => string> = {
    silver: m.plugin_lol_rarity_silver, gold: m.plugin_lol_rarity_gold, prismatic: m.plugin_lol_rarity_prismatic,
  };

  function basis(c: AugmentOverlayData): string {
    const parts: string[] = [];
    if (c.rarityRank && c.rarityTotal) {
      parts.push(m.plugin_lol_overlay_rarity_rank({ rarity: RARITY[c.rarity]?.() ?? "", rank: c.rarityRank, total: c.rarityTotal }));
    }
    if (c.sampleWarning && c.thin !== "tooFew") parts.push(m.plugin_lol_low_sample());
    return parts.join(" · ");
  }
</script>

{#if card}
  {@const s = card.score}
  <div class="lol-overlay chip" class:best={card.verdict === "best"} class:faded={s?.lowSample} data-testid="lol-overlay-augment"
    data-verdict={card.verdict} data-grade={card.grade ?? ""} data-augment={card.augment}>
    <div class="head">
      {#if card.grade}<span class="lol-grade" data-grade={card.grade}>{card.grade}</span>{/if}
      <span class="verdict" data-testid="lol-overlay-verdict">{verdictText(card)}</span>
    </div>
    <div class="stat">
      {#if s}
        <span class="delta" class:up={s.delta > 0} class:down={s.delta < 0}>{signed(s.delta)}</span>
        <span>· {rate(s.winRate)} · {m.plugin_lol_overlay_games({ games: compact(s.games) })}</span>
      {:else if card.rare}
        <span>{m.plugin_lol_overlay_rare()}</span>
      {:else}
        <span>{m.plugin_lol_overlay_no_data()}</span>
      {/if}
    </div>
    <div class="foot">
      <span class="basis">{basis(card)}</span>
      <span class="credit">{card.attribution}</span>
    </div>
  </div>
{/if}

<style>
  .chip {
    padding: 3cqh 4cqw; display: flex; flex-direction: column; justify-content: center; gap: 3cqh;
    font-size: clamp(10px, 24cqh, 24px); line-height: 1.15;
  }
  .chip.best { border-color: transparent; box-shadow: inset 0 0 0 2px var(--ov-text); }
  .chip.faded { opacity: 0.7; }
  .head, .stat, .foot { display: flex; align-items: center; gap: 0.35em; white-space: nowrap; overflow: hidden; }
  .verdict { font-weight: 700; overflow: hidden; text-overflow: ellipsis; color: var(--ov-text); }
  [data-verdict="best"] .verdict { color: var(--ov-win); }
  [data-verdict="reroll"] .verdict { color: var(--ov-loss); }
  [data-verdict="noData"] .verdict { color: var(--ov-text-3); }
  .stat { color: var(--ov-text-2); font-size: 0.7em; }
  .delta { font-weight: 700; color: var(--ov-text); }
  .delta.up { color: var(--ov-win); }
  .delta.down { color: var(--ov-loss); }
  .foot { font-size: 0.54em; color: var(--ov-text-2); }
  .basis { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .credit { margin-left: auto; min-width: 0; color: var(--ov-text-3); overflow: hidden; text-overflow: ellipsis; }
</style>
