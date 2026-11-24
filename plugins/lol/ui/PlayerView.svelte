<script lang="ts">
  // 战绩（`ui://kotone/lol/player`，`lol.player` 的页面，照 LeagueAkari 的战绩页）：段位、近况、常玩的英雄、最近的对局；
  // 点一局看这局 10 个人。上面能搜别人（名字#编号，或这局里的英雄、玩家名）。
  // 参数：`puuid`（从这局点进来）或 `player`；都没有 = 自己。
  import { onMount } from "svelte";
  import { EyeOff, LoaderCircle, RotateCw, Search } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { companionName } from "$lib/stores/companion";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_GAME, statusText, type PlayerData } from "./types";
  import { duration, kda, num, pct, when } from "./format";

  let { params, data, bridge }: PluginViewProps = $props();
  const initial = () => (data as PlayerData | null) ?? null;
  let player = $state<PlayerData | null>(initial());
  let snapshot = $state(initial() !== null);
  let status = $state("");
  let error = $state("");
  let loading = $state(false);
  let query = $state("");

  /** 这一页查的是谁（刷新时照这个查）。 */
  let who = $state<Record<string, unknown>>({});

  async function load(args: Record<string, unknown>) {
    loading = true;
    error = "";
    try {
      const result = await bridge.call("lol.player", args);
      if (result.structuredContent) {
        player = result.structuredContent as PlayerData;
        who = player.puuid ? { puuid: player.puuid } : args;
        status = "";
        snapshot = false;
      } else {
        status = statusText(result.content);
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    const p = params as { puuid?: unknown; player?: unknown; riotId?: unknown };
    const args: Record<string, unknown> = {};
    if (typeof p.puuid === "string" && p.puuid) args.puuid = p.puuid;
    else if (typeof p.player === "string" && p.player) args.player = p.player;
    else if (typeof p.riotId === "string" && p.riotId) args.player = p.riotId;
    who = player?.puuid ? { puuid: player.puuid } : args;
    if (!player) void load(args);
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    const q = query.trim();
    void load(q ? { player: q } : {});
  }
</script>

<div class="space-y-3" data-testid="lol-player-page">
  <form class="flex items-center gap-1.5" onsubmit={search} data-testid="lol-player-search">
    <div class="flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-lg border border-hairline bg-background px-2 focus-within:ring-1 focus-within:ring-primary/50">
      <Search class="size-3.5 shrink-0 text-muted-foreground" />
      <input bind:value={query} placeholder={m.plugin_lol_search_placeholder()} aria-label={m.plugin_lol_search_placeholder()}
        class="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground" data-testid="lol-player-search-input" />
    </div>
    <Button type="submit" size="sm" class="h-8 px-2.5 text-xs" disabled={loading} data-testid="lol-player-search-go">{query.trim() ? m.plugin_lol_search() : m.plugin_lol_search_me()}</Button>
    <Button type="button" size="icon" variant="outline" class="size-8" aria-label={m.plugin_lol_refresh()} title={m.plugin_lol_refresh()}
      disabled={loading} data-testid="lol-refresh" onclick={() => void load(who)}>
      {#if loading}<LoaderCircle class="size-3.5 animate-spin" />{:else}<RotateCw class="size-3.5" />{/if}
    </Button>
  </form>
  {#if snapshot}<p class="m-0 text-[11px] text-muted-foreground" data-testid="lol-snapshot">{m.plugin_lol_snapshot({ companion: $companionName })}</p>{/if}
  {#if error}<p role="alert" class="m-0 text-xs text-destructive">{m.plugin_lol_failed({ error })}</p>{/if}
  {#if status}<p class="m-0 rounded-lg bg-muted/50 px-3 py-2 text-xs text-foreground/85" data-testid="lol-status">{status}</p>{/if}
  {#if player}
    {@const s = player.summary}
    <header class="space-y-0.5">
      <p class="m-0 flex min-w-0 items-center gap-2">
        <span class="min-w-0 truncate text-sm font-medium text-foreground" data-testid="lol-player-name">{player.riotId}</span>
        {#if player.rank}<span class="shrink-0 text-xs text-muted-foreground">{player.rank}</span>{/if}
        {#if player.hidden}<span class="flex shrink-0 items-center gap-0.5 text-[11px] text-warning"><EyeOff class="size-3" />{m.plugin_lol_hidden()}</span>{/if}
      </p>
      {#if s && s.games > 0}
        <p class="m-0 text-xs text-foreground/85">{m.plugin_lol_record({ games: s.games, wins: s.wins, losses: s.losses })} · {pct(s.wins, s.games)} · {m.plugin_lol_kda({ kda: s.kda, deaths: s.deaths })}</p>
      {:else}
        <p class="m-0 text-xs text-muted-foreground">{m.plugin_lol_no_history()}{#if player.error} · {player.error}{/if}</p>
      {/if}
    </header>
    {#if player.champions.length}
      <section aria-label={m.plugin_lol_top_champions()}>
        <h4 class="mt-0 mb-1 text-xs font-medium text-muted-foreground">{m.plugin_lol_top_champions()}</h4>
        <div class="flex flex-wrap gap-1.5">
          {#each player.champions as c (c.champion.id)}
            <span class="rounded-md bg-muted/60 px-2 py-1 text-xs"><span class="font-medium">{c.champion.short}</span>
              <span class="text-muted-foreground">{m.plugin_lol_champion_record({ games: c.games, wins: c.wins })}</span></span>
          {/each}
        </div>
      </section>
    {/if}
    <section aria-label={m.plugin_lol_games()}>
      <h4 class="mt-0 mb-1 text-xs font-medium text-muted-foreground">{m.plugin_lol_games()}</h4>
      <ul class="m-0 grid list-none gap-1 p-0" data-testid="lol-player-games">
        {#each player.games as g (g.gameId)}
          <li>
            <button type="button" class="flex w-full min-w-0 items-center gap-2 rounded-lg border border-hairline px-2.5 py-1.5 text-left text-xs transition-colors hover:border-primary/40 hover:bg-muted/40"
              data-testid="lol-player-game" data-game-id={g.gameId} onclick={() => bridge.openPage(PAGE_GAME, { gameId: g.gameId })}>
              <span class="w-4 shrink-0 font-medium {g.win ? 'text-success' : 'text-destructive'}">{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</span>
              <span class="w-16 shrink-0 truncate font-medium">{g.champion.short}</span>
              <span class="w-14 shrink-0 tabular-nums">{kda(g.kills, g.deaths, g.assists)}</span>
              <span class="min-w-0 flex-1 truncate text-muted-foreground">{g.queue} · {duration(g.durationS)} · {m.plugin_lol_damage({ damage: num(g.damage), dpm: num(g.dpm) })}</span>
              <span class="shrink-0 text-[11px] text-muted-foreground">{when(g.createdMs)}</span>
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {:else if loading}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_lol_loading()}</p>
  {/if}
</div>
