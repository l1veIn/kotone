<script lang="ts">
  // 这局的聊天卡片（小尺寸、只读快照）：对面 5 个人一行一个（近况、标签），突破口一句，我方收成一行。
  import { EyeOff } from "@lucide/svelte";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { MatchData } from "./types";
  import { pct } from "./format";

  let { data }: PluginViewProps = $props();
  const match = $derived(data as MatchData | null);
  const weak = $derived(match?.enemies.filter((p) => p.weak) ?? []);
</script>

{#if match}
  <div class="space-y-1.5 text-xs" data-testid="lol-match-card">
    <p class="m-0 text-[11px] text-muted-foreground">{match.mode} · {m.plugin_lol_enemies()}</p>
    <ul class="m-0 grid list-none gap-0.5 p-0">
      {#each match.enemies as p (p.puuid || p.champion.id)}
        <li class="flex min-w-0 items-center gap-2 {p.weak ? 'text-destructive' : 'text-foreground/90'}" data-testid="lol-match-card-enemy">
          <span class="w-16 shrink-0 truncate font-medium">{p.champion.short}</span>
          <span class="min-w-0 flex-1 truncate text-muted-foreground">{p.name}</span>
          {#if p.hidden}<EyeOff class="size-3 shrink-0 text-warning" aria-label={m.plugin_lol_hidden()} />{/if}
          {#if p.summary && p.summary.games > 0}
            <span class="shrink-0 tabular-nums">{p.summary.wins}/{p.summary.games} · {pct(p.summary.wins, p.summary.games)}</span>
          {:else}
            <span class="shrink-0 text-muted-foreground">{p.pending ? m.plugin_lol_pending() : m.plugin_lol_no_history()}</span>
          {/if}
        </li>
      {/each}
    </ul>
    {#each weak as p (p.puuid)}
      <p class="m-0 text-destructive" data-testid="lol-match-card-weak">{m.plugin_lol_card_weak({ who: p.champion.short, why: (p.weak ?? []).join(" · ") })}</p>
    {/each}
    <p class="m-0 truncate text-[11px] text-muted-foreground">{m.plugin_lol_card_allies({ names: match.allies.map((p) => p.champion.short).join(" · ") })}</p>
  </div>
{/if}
