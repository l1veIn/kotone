<script lang="ts">
  // 战绩的聊天卡片（小尺寸、只读快照）：名字、段位、近况、最近 5 局。
  import { EyeOff } from "@lucide/svelte";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { PlayerData } from "./types";
  import { kda, pct } from "./format";

  let { data }: PluginViewProps = $props();
  const player = $derived(data as PlayerData | null);
  const RECENT = 5;
</script>

{#if player}
  {@const s = player.summary}
  <div class="space-y-1 text-xs" data-testid="lol-player-card">
    <p class="m-0 flex min-w-0 items-center gap-2">
      <span class="min-w-0 truncate font-medium text-foreground">{player.riotId}</span>
      {#if player.rank}<span class="shrink-0 text-muted-foreground">{player.rank}</span>{/if}
      {#if player.hidden}<EyeOff class="size-3 shrink-0 text-warning" aria-label={m.plugin_lol_hidden()} />{/if}
    </p>
    {#if s && s.games > 0}
      <p class="m-0 text-foreground/85">{m.plugin_lol_record({ games: s.games, wins: s.wins, losses: s.losses })} · {pct(s.wins, s.games)} · {m.plugin_lol_kda({ kda: s.kda, deaths: s.deaths })}</p>
    {/if}
    <ul class="m-0 grid list-none gap-0.5 p-0">
      {#each player.games.slice(0, RECENT) as g (g.gameId)}
        <li class="flex min-w-0 items-center gap-2" data-testid="lol-player-card-game">
          <span class="w-4 shrink-0 font-medium {g.win ? 'text-success' : 'text-destructive'}">{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</span>
          <span class="w-16 shrink-0 truncate">{g.champion.short}</span>
          <span class="shrink-0 tabular-nums">{kda(g.kills, g.deaths, g.assists)}</span>
          <span class="min-w-0 truncate text-muted-foreground">{g.queue}</span>
        </li>
      {/each}
    </ul>
    {#if player.games.length > RECENT}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_lol_card_more({ count: player.games.length - RECENT })}</p>{/if}
  </div>
{/if}
