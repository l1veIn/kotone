<script lang="ts">
  // 强度榜的聊天卡片（小尺寸、只读快照）：峡谷每一路前三，海斗前五。最下面写数据从哪来。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { TierData } from "./types";
  import { rate } from "./format";
  import { sourcesLine } from "./stats";

  let { data }: PluginViewProps = $props();
  const tier = $derived(data as TierData | null);
</script>

{#if tier}
  <div class="space-y-1 text-xs" data-testid="lol-tier-card" data-mode={tier.mode}>
    {#if tier.mode === "rift"}
      {#each tier.laneNames as lane (lane.id)}
        <p class="m-0 flex min-w-0 gap-2" data-testid="lol-tier-card-lane">
          <span class="w-8 shrink-0 text-muted-foreground">{lane.name}</span>
          <span class="min-w-0 truncate text-foreground/90">{(tier.lanes[lane.id] ?? []).slice(0, 3).map((r) => `${r.champion.short} ${r.tier}`).join(" · ")}</span>
        </p>
      {/each}
    {:else}
      <p class="m-0 text-[11px] text-muted-foreground">{m.plugin_lol_mode_mayhem()}</p>
      {#each tier.rows.slice(0, 5) as row (row.champion.id)}
        <p class="m-0 flex min-w-0 gap-2" data-testid="lol-tier-card-row">
          <span class="w-4 shrink-0 text-right text-muted-foreground tabular-nums">{row.rank ?? ""}</span>
          <span class="min-w-0 flex-1 truncate">{row.champion.short}</span>
          <span class="shrink-0 tabular-nums">{rate(row.winRate)}</span>
        </p>
      {/each}
    {/if}
    <p class="m-0 truncate text-[10px] text-muted-foreground">{m.plugin_lol_sources({ sources: sourcesLine(tier.sources) })}</p>
  </div>
{/if}
