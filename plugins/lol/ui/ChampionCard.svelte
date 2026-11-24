<script lang="ts">
  // 英雄的聊天卡片（小尺寸、只读快照）：峡谷——强度一行、难打的三个、核心三件；海斗——名次一行、最好的三个海克斯。
  // 最下面一行写数据从哪来（aramhex-data 要署名）。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { ChampionData } from "./types";
  import { rate, signed } from "./format";
  import { rarityClass, sourcesLine } from "./stats";

  let { data }: PluginViewProps = $props();
  const champion = $derived(data as ChampionData | null);
</script>

{#if champion}
  <div class="space-y-1 text-xs" data-testid="lol-champion-card" data-mode={champion.mode}>
    <p class="m-0 flex min-w-0 items-baseline gap-2">
      <span class="min-w-0 truncate font-medium text-foreground">{champion.champion.label}</span>
      <span class="shrink-0 text-[11px] text-muted-foreground">{champion.mode === "rift" ? `${m.plugin_lol_mode_rift()} · ${champion.laneName}` : m.plugin_lol_mode_mayhem()}</span>
    </p>
    {#if champion.mode === "rift"}
      {#if champion.tier}
        <p class="m-0 text-foreground/85"><span class="mr-1 font-medium text-primary">{champion.tier.tier}</span>{m.plugin_lol_tier_stats({ win: rate(champion.tier.winRate), pick: rate(champion.tier.pickRate), ban: rate(champion.tier.banRate) })}</p>
      {/if}
      {#if champion.badAgainst.length}
        <p class="m-0 truncate">{m.plugin_lol_card_bad({ list: champion.badAgainst.slice(0, 3).map((x) => `${x.champion.short} ${rate(x.winRate)}`).join(" · ") })}</p>
      {/if}
      {#if champion.build.core[0]}
        <p class="m-0 truncate">{m.plugin_lol_card_core({ items: champion.build.core[0].items.map((i) => i.name).join(" → ") })}</p>
      {/if}
    {:else}
      <p class="m-0 text-foreground/85">{m.plugin_lol_mayhem_rank({ rank: champion.rank ?? "—", win: rate(champion.winRate), pick: rate(champion.pickRate) })}</p>
      <ul class="m-0 grid list-none gap-0.5 p-0">
        {#each champion.augments.slice(0, 3) as a (a.id)}
          <li class="flex min-w-0 items-center gap-2" data-testid="lol-champion-card-augment">
            <span class="size-1.5 shrink-0 rounded-full {rarityClass(a.rarity)}"></span>
            <span class="min-w-0 flex-1 truncate">{a.name}</span>
            <span class="shrink-0 tabular-nums {a.delta >= 0 ? 'text-success' : 'text-destructive'}">{signed(a.delta)}</span>
          </li>
        {/each}
      </ul>
    {/if}
    <p class="m-0 truncate text-[10px] text-muted-foreground" data-testid="lol-champion-card-sources">{m.plugin_lol_sources({ sources: sourcesLine(champion.sources) })}</p>
  </div>
{/if}
