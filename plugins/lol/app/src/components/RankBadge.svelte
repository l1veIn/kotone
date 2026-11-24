<script lang="ts">
  // 段位：客户端的小徽章 + 字（“钻石 II · 45 点”；卡片里短写“钻 II”）。没定级只写“未定级”不画图；
  // 徽章挂了只留字。没有新字段（旧数据）时用旧的 `rank` 字符串。
  import { crestPath, sources } from "../assets";
  import { rankText, tierName } from "../format";
  import { m } from "../i18n.svelte";
  import type { Ranked } from "../types";

  let {
    ranked,
    legacy = null,
    short = false,
    size = 16,
    hideText = false,
  }: { ranked: Ranked | null | undefined; legacy?: string | null; short?: boolean; size?: number; hideText?: boolean } =
    $props();

  const crest = $derived(ranked?.tier ? ranked.crest || crestPath(ranked.tier) : "");
  const list = $derived(sources(crest));
  let attempt = $state(0);
  const text = $derived(ranked ? rankText(ranked, short) : (legacy ?? ""));
  const full = $derived(ranked ? rankText(ranked, false) : (legacy ?? ""));
</script>

{#if ranked || legacy}
  <span class="rank" title={ranked?.highestTier ? `${full} · ${m.plugin_lol_rank_highest({ tier: tierName(ranked.highestTier) })}` : full}>
    {#if list[attempt]}
      <img src={list[attempt]} alt="" width={size} height={size} style:width="{size}px" style:height="{size}px" referrerpolicy="no-referrer" onerror={() => (attempt += 1)} />
    {/if}
    <span class="rt" class:sr-only={hideText}>{text}</span>
  </span>
{/if}
