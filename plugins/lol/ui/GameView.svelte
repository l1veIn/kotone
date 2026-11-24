<script lang="ts">
  // 单局详情（`ui://kotone/lol/game`，只给页面用的 `lol.game`）：两边 10 个人的英雄、KDA、伤害、经济、补刀、装备、海克斯。
  // 点一个人看他的战绩。参数：`gameId`。
  import { onMount } from "svelte";
  import { LoaderCircle } from "@lucide/svelte";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_PLAYER, statusText, type GameData } from "./types";
  import { duration, kda, num, when } from "./format";

  let { params, data, bridge }: PluginViewProps = $props();
  const initial = () => (data as GameData | null) ?? null;
  let game = $state<GameData | null>(initial());
  let status = $state("");
  let error = $state("");
  let loading = $state(false);

  onMount(async () => {
    if (game) return;
    const gameId = Number((params as { gameId?: unknown }).gameId);
    loading = true;
    try {
      const result = await bridge.call("lol.game", { gameId });
      if (result.structuredContent) game = result.structuredContent as GameData;
      else status = statusText(result.content);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });
</script>

<div class="space-y-3" data-testid="lol-game-page">
  {#if error}<p role="alert" class="m-0 text-xs text-destructive">{m.plugin_lol_failed({ error })}</p>{/if}
  {#if status}<p class="m-0 rounded-lg bg-muted/50 px-3 py-2 text-xs text-foreground/85" data-testid="lol-status">{status}</p>{/if}
  {#if game}
    <p class="m-0 text-xs text-muted-foreground" data-testid="lol-game-header">{game.queue} · {duration(game.durationS)} · {when(game.createdMs)}</p>
    {#each game.teams as team (team.teamId)}
      <section aria-label={team.win ? m.plugin_lol_team_win() : m.plugin_lol_team_loss()} data-testid="lol-game-team" data-win={team.win ? "true" : "false"}>
        <h4 class="mt-0 mb-1 text-xs font-medium {team.win ? 'text-success' : 'text-destructive'}">{team.win ? m.plugin_lol_team_win() : m.plugin_lol_team_loss()}</h4>
        <div class="overflow-x-auto">
          <table class="w-full min-w-[30rem] border-collapse text-xs">
            <thead>
              <tr class="text-left text-[11px] whitespace-nowrap text-muted-foreground">
                <th class="py-1 pr-2 font-normal">{m.plugin_lol_col_player()}</th>
                <th class="py-1 pr-2 font-normal">KDA</th>
                <th class="py-1 pr-2 font-normal">{m.plugin_lol_col_damage()}</th>
                <th class="py-1 pr-2 font-normal">{m.plugin_lol_col_gold()}</th>
                <th class="py-1 pr-2 font-normal">{m.plugin_lol_col_cs()}</th>
                <th class="py-1 font-normal">{m.plugin_lol_col_items()}</th>
              </tr>
            </thead>
            <tbody>
              {#each team.players as p (p.puuid || p.champion.id)}
                <tr class="border-t border-hairline hover:bg-muted/40" data-testid="lol-game-player" data-puuid={p.puuid}>
                  <td class="py-1.5 pr-2">
                    <button type="button" class="text-left hover:text-primary disabled:cursor-default" disabled={!p.puuid}
                      data-testid="lol-game-player-open" onclick={() => bridge.openPage(PAGE_PLAYER, { puuid: p.puuid })}>
                      <span class="font-medium">{p.champion.short}</span>
                      <span class="block max-w-[9rem] truncate text-[11px] text-muted-foreground" title={p.riotId}>{p.riotId}</span>
                    </button>
                  </td>
                  <td class="py-1.5 pr-2 tabular-nums">{kda(p.kills, p.deaths, p.assists)}</td>
                  <td class="py-1.5 pr-2 tabular-nums">{num(p.damage)}</td>
                  <td class="py-1.5 pr-2 tabular-nums">{num(p.gold)}</td>
                  <td class="py-1.5 pr-2 tabular-nums">{p.cs}</td>
                  <td class="py-1.5 text-[11px] text-muted-foreground">
                    {p.items.map((i) => i.name || i.id).join(" · ")}
                    {#if p.augments.length}<span class="block" title={m.plugin_lol_augments()}>{p.augments.map((a) => a.name || a.id).join(" · ")}</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/each}
  {:else if loading}
    <p class="m-0 flex items-center gap-1.5 text-xs text-muted-foreground"><LoaderCircle class="size-3 animate-spin" />{m.plugin_lol_loading()}</p>
  {/if}
</div>
