<script lang="ts">
  // 这局（`ui://kotone/lol/match`，`lol.scout` 的页面，照 LeagueAkari 的“对局”页）：两边各 5 个人——英雄、玩家、段位、
  // 隐藏战绩、开黑、近况、标签、最近几局的输赢；对面的突破口标出来并写原因。点一个人看他的战绩。
  // 从聊天卡片点开时先显示她查的那一刻的样子，“刷新”看现在的。
  import { onMount } from "svelte";
  import { EyeOff, LoaderCircle, RotateCw, Target } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { companionName } from "$lib/stores/companion";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_PLAYER, statusText, type MatchData, type MatchPlayer } from "./types";
  import { kda, partyColor, pct } from "./format";

  let { data, bridge }: PluginViewProps = $props();
  const initial = () => (data as MatchData | null) ?? null;
  let match = $state<MatchData | null>(initial());
  let snapshot = $state(initial() !== null);
  let status = $state("");
  let error = $state("");
  let loading = $state(false);

  async function load() {
    loading = true;
    error = "";
    try {
      const result = await bridge.call("lol.scout");
      if (result.structuredContent) {
        match = result.structuredContent as MatchData;
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

  onMount(() => { if (!match) void load(); });

  function open(p: MatchPlayer) {
    if (p.puuid) bridge.openPage(PAGE_PLAYER, { puuid: p.puuid });
  }
</script>

{#snippet row(p: MatchPlayer, team: MatchPlayer[])}
  {@const s = p.summary}
  {@const color = p.party !== null ? partyColor(p.party, team.map((x) => x.party)) : ""}
  <li>
    <button type="button" class="w-full rounded-lg border px-2.5 py-2 text-left transition-colors hover:border-primary/40 hover:bg-muted/40
      {p.weak ? 'border-destructive/40 bg-destructive/5' : 'border-hairline'}"
      data-testid="lol-match-player" data-puuid={p.puuid} data-weak={p.weak ? "true" : "false"} onclick={() => open(p)}>
      <div class="flex min-w-0 items-center gap-2">
        {#if color}<span class="size-2 shrink-0 rounded-full {color}" title={m.plugin_lol_party()} data-testid="lol-match-party"></span>{/if}
        <span class="shrink-0 text-[13px] font-medium text-foreground">{p.champion.short}</span>
        <span class="min-w-0 truncate text-xs text-muted-foreground" title={p.riotId || p.name}>{p.riotId || p.name}</span>
        {#if p.me}<span class="shrink-0 rounded bg-primary/15 px-1 text-[10px] text-primary">{m.plugin_lol_me()}</span>{/if}
        {#if p.hidden}<span class="flex shrink-0 items-center gap-0.5 text-[11px] text-warning" data-testid="lol-match-hidden"><EyeOff class="size-3" />{m.plugin_lol_hidden()}</span>{/if}
        <span class="ml-auto shrink-0 text-[11px] text-muted-foreground">{p.rank ?? ""}</span>
      </div>
      {#if p.pending}
        <p class="m-0 mt-1 text-xs text-muted-foreground">{m.plugin_lol_pending()}</p>
      {:else if s && s.games > 0}
        <p class="m-0 mt-1 text-xs text-foreground/85">
          {m.plugin_lol_record({ games: s.games, wins: s.wins, losses: s.losses })} · {pct(s.wins, s.games)} · {m.plugin_lol_kda({ kda: s.kda, deaths: s.deaths })}
        </p>
        {#if s.modeGames > 0}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_lol_mode_record({ games: s.modeGames, wins: s.modeWins, dpm: s.modeDpm })}</p>{/if}
        <div class="mt-1 flex flex-wrap items-center gap-1">
          <span class="flex gap-0.5" aria-label={m.plugin_lol_recent({ count: p.recent.length })}>
            {#each p.recent as g (g.gameId)}
              <span class="h-2.5 w-1.5 rounded-[2px] {g.win ? 'bg-success/80' : 'bg-destructive/70'}" title={`${g.win ? m.plugin_lol_win() : m.plugin_lol_loss()} ${kda(g.kills, g.deaths, g.assists)}`}></span>
            {/each}
          </span>
          {#each p.tags as tag, i (i)}<span class="rounded bg-muted px-1.5 text-[10px] leading-4 text-muted-foreground">{typeof tag === "string" ? tag : tag.text}</span>{/each}
        </div>
      {:else}
        <p class="m-0 mt-1 text-xs text-muted-foreground">{m.plugin_lol_no_history()}{#if p.error} · {p.error}{/if}</p>
      {/if}
      {#if p.weak}
        <p class="m-0 mt-1 flex items-start gap-1 text-xs text-destructive" data-testid="lol-match-weak">
          <Target class="mt-0.5 size-3 shrink-0" /><span><span class="font-medium">{m.plugin_lol_weak()}</span> · {p.weak.join(" · ")}</span>
        </p>
      {/if}
    </button>
  </li>
{/snippet}

<div class="space-y-3" data-testid="lol-match-page">
  <div class="flex items-center gap-2">
    <p class="m-0 min-w-0 flex-1 truncate text-xs text-muted-foreground">{match?.mode ?? ""}</p>
    <Button size="sm" variant="outline" class="h-7 gap-1 px-2 text-xs" data-testid="lol-refresh" disabled={loading} onclick={() => void load()}>
      {#if loading}<LoaderCircle class="size-3 animate-spin" />{:else}<RotateCw class="size-3" />{/if}{m.plugin_lol_refresh()}
    </Button>
  </div>
  {#if snapshot}<p class="m-0 text-[11px] text-muted-foreground" data-testid="lol-snapshot">{m.plugin_lol_snapshot({ companion: $companionName })}</p>{/if}
  {#if error}<p role="alert" class="m-0 text-xs text-destructive">{m.plugin_lol_failed({ error })}</p>{/if}
  {#if status}<p class="m-0 rounded-lg bg-muted/50 px-3 py-2 text-xs text-foreground/85" data-testid="lol-status">{status}</p>{/if}
  {#if match}
    {#if match.random}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_lol_random_note()}</p>{/if}
    {#if match.waiting > 0}<p class="m-0 text-[11px] text-warning">{m.plugin_lol_waiting({ count: match.waiting })}</p>{/if}
    <section aria-label={m.plugin_lol_enemies()}>
      <h4 class="mt-0 mb-1.5 text-xs font-medium text-muted-foreground">{m.plugin_lol_enemies()}</h4>
      <ul class="m-0 grid list-none gap-1.5 p-0" data-testid="lol-match-enemies">
        {#each match.enemies as p (p.puuid || p.champion.id)}{@render row(p, match.enemies)}{/each}
      </ul>
    </section>
    <section aria-label={m.plugin_lol_allies()}>
      <h4 class="mt-0 mb-1.5 text-xs font-medium text-muted-foreground">{m.plugin_lol_allies()}</h4>
      <ul class="m-0 grid list-none gap-1.5 p-0" data-testid="lol-match-allies">
        {#each match.allies as p (p.puuid || p.champion.id)}{@render row(p, match.allies)}{/each}
      </ul>
    </section>
  {:else if loading}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_lol_loading()}</p>
  {/if}
</div>
