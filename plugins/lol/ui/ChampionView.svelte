<script lang="ts">
  // 英雄（`ui://kotone/lol/champion`，`lol.champion` 的页面，照 LeagueAkari / OP.GG 的英雄页）：
  // 峡谷——这一路的强度、克制、出装、符文、加点、召唤师技能（腾讯 101）；开了 OP.GG 时多一块全球数据；
  // 海斗——英雄榜名次、最搭的队友、最好的海克斯（腾讯 101 + aramhex-data，署名、CC BY-NC）。
  // 能搜别的英雄、切峡谷 / 海斗、切分路；点一个英雄进它的页面。数据来源写在最下面。
  import { onMount } from "svelte";
  import { ExternalLink, LoaderCircle, RotateCw, Search } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { companionName } from "$lib/stores/companion";
  import { openExternal } from "$lib/ipc";
  import { FROM_PAGE, type PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import {
    ARAMHEX_URL, PAGE_CHAMPION, statusText,
    type BuildOption, type ChampionData, type ChampionMayhem, type ChampionRift, type Named, type StatChampion,
  } from "./types";
  import { compact, rate, signed } from "./format";
  import { rarityClass, sourcesLine } from "./stats";

  let { params, data, bridge }: PluginViewProps = $props();
  const initial = () => (data as ChampionData | null) ?? null;
  let champion = $state<ChampionData | null>(initial());
  // 卡片点开的是她查的那一刻的样子；页面自己查的（搜、点进来、返回）不是
  const fromPage = () => Boolean(params[FROM_PAGE]);
  let snapshot = $state(initial() !== null && !fromPage());
  let query = $state("");
  let status = $state("");
  let error = $state("");
  let loading = $state(false);

  const LANES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "SUPPORT"] as const;
  const laneLabel = (id: string) => ({
    TOP: m.plugin_lol_lane_top(), JUNGLE: m.plugin_lol_lane_jungle(), MIDDLE: m.plugin_lol_lane_middle(),
    BOTTOM: m.plugin_lol_lane_bottom(), SUPPORT: m.plugin_lol_lane_support(),
  })[id] ?? id;

  async function load(args: Record<string, unknown>) {
    loading = true;
    error = "";
    try {
      const result = await bridge.call("lol.champion", args);
      if (result.structuredContent) {
        champion = result.structuredContent as ChampionData;
        status = "";
        snapshot = false;
        // 记下来：从下一页返回时还是这个英雄
        bridge.replace({ ...args, [FROM_PAGE]: true }, champion);
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
    const name = typeof params.champion === "string" ? params.champion : "";
    const { [FROM_PAGE]: _, ...args } = params;
    if (!champion && name) void load(args);
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    const name = query.trim();
    if (!name) return;
    void load(champion?.mode === "mayhem" ? { champion: name, mode: "mayhem" } : { champion: name });
  }
  const current = $derived(champion?.champion.short ?? "");
  function switchMode(mode: "rift" | "mayhem") {
    if (current) void load({ champion: current, mode });
  }
  function switchLane(lane: string) {
    if (current) void load({ champion: current, mode: "rift", lane });
  }
  function refresh() {
    if (!champion) return;
    void load(champion.mode === "rift" ? { champion: current, mode: "rift", lane: champion.lane } : { champion: current, mode: "mayhem" });
  }
  /** 点一个英雄：进它的页面（同一种模式），能返回。 */
  async function openChampion(c: StatChampion) {
    const mode = champion?.mode ?? "rift";
    try {
      const result = await bridge.call("lol.champion", { champion: c.short, mode });
      if (result.structuredContent) bridge.openPage(PAGE_CHAMPION, { champion: c.short, mode, [FROM_PAGE]: true }, result.structuredContent);
      else status = statusText(result.content);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  const names = (list: Named[]) => list.map((x) => x.name).join(" · ");
</script>

{#snippet options(label: string, list: BuildOption[], testid: string)}
  {#if list.length}
    <div data-testid={testid}>
      <p class="m-0 mb-1 text-[11px] font-medium text-muted-foreground">{label}</p>
      <ul class="m-0 grid list-none gap-1 p-0">
        {#each list as option, i (i)}
          <li class="flex min-w-0 flex-wrap items-baseline gap-x-2 rounded-md bg-muted/40 px-2 py-1 text-xs">
            <span class="min-w-0 text-foreground">{names(option.items)}</span>
            <span class="ml-auto shrink-0 text-[11px] text-muted-foreground tabular-nums">{m.plugin_lol_win_pick({ win: rate(option.winRate), pick: rate(option.pickRate) })}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
{/snippet}

{#snippet matchups(label: string, list: { champion: StatChampion; winRate: number | null }[], tone: string, testid: string)}
  {#if list.length}
    <div class="min-w-0 flex-1" data-testid={testid}>
      <p class="m-0 mb-1 text-[11px] font-medium text-muted-foreground">{label}</p>
      <ul class="m-0 grid list-none gap-0.5 p-0">
        {#each list as row (row.champion.id)}
          <li>
            <button type="button" class="flex w-full items-center gap-2 rounded-md px-1.5 py-0.5 text-left text-xs hover:bg-muted/50"
              data-testid="lol-champion-link" onclick={() => void openChampion(row.champion)}>
              <span class="min-w-0 flex-1 truncate text-foreground">{row.champion.short}</span>
              <span class="shrink-0 tabular-nums {tone}">{rate(row.winRate)}</span>
            </button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
{/snippet}

{#snippet rift(c: ChampionRift)}
  <div class="space-y-1">
    <div class="flex flex-wrap gap-1" role="group" aria-label={m.plugin_lol_lane()}>
      {#each LANES as lane (lane)}
        <button type="button" data-testid={`lol-champion-lane-${lane}`} aria-pressed={c.lane === lane}
          class="h-6 rounded-md px-2 text-[11px] transition-colors {c.lane === lane ? 'bg-primary/15 font-medium text-primary' : 'text-muted-foreground hover:bg-muted/60 hover:text-foreground'}"
          onclick={() => switchLane(lane)}>{laneLabel(lane)}</button>
      {/each}
    </div>
    {#if c.tier}
      <p class="m-0 text-xs text-foreground/90" data-testid="lol-champion-tier">
        <span class="mr-1 rounded bg-primary/15 px-1.5 font-medium text-primary">{c.tier.tier}</span>
        {m.plugin_lol_tier_stats({ win: rate(c.tier.winRate), pick: rate(c.tier.pickRate), ban: rate(c.tier.banRate) })}
      </p>
    {/if}
  </div>
  <div class="flex gap-3">
    {@render matchups(m.plugin_lol_bad_against(), c.badAgainst, "text-destructive", "lol-champion-bad")}
    {@render matchups(m.plugin_lol_good_against(), c.goodAgainst, "text-success", "lol-champion-good")}
  </div>
  <section class="space-y-2" aria-label={m.plugin_lol_build()} data-testid="lol-champion-build">
    <h4 class="m-0 text-xs font-medium text-foreground">{m.plugin_lol_build()}</h4>
    {@render options(m.plugin_lol_build_core(), c.build.core, "lol-champion-core")}
    {@render options(m.plugin_lol_build_boots(), c.build.boots, "lol-champion-boots")}
    {@render options(m.plugin_lol_build_starting(), c.build.starting, "lol-champion-starting")}
    {@render options(m.plugin_lol_build_fourth(), c.build.fourth, "lol-champion-fourth")}
    {@render options(m.plugin_lol_build_fifth(), c.build.fifth, "lol-champion-fifth")}
    {@render options(m.plugin_lol_build_sixth(), c.build.sixth, "lol-champion-sixth")}
  </section>
  {#if c.runes.length}
    <section class="space-y-1" aria-label={m.plugin_lol_runes()} data-testid="lol-champion-runes">
      <h4 class="m-0 text-xs font-medium text-foreground">{m.plugin_lol_runes()}</h4>
      {#each c.runes as page, i (i)}
        <div class="rounded-md bg-muted/40 px-2 py-1.5 text-xs">
          <p class="m-0 text-foreground">{names(page.primary)}</p>
          {#if page.secondary.length}<p class="m-0 text-foreground/80">{names(page.secondary)}</p>{/if}
          {#if page.shards.length}<p class="m-0 text-[11px] text-muted-foreground">{names(page.shards)}</p>{/if}
          <p class="m-0 text-[11px] text-muted-foreground tabular-nums">{m.plugin_lol_win_pick({ win: rate(page.winRate), pick: rate(page.pickRate) })} · {m.plugin_lol_games_count({ games: compact(page.games) })}</p>
        </div>
      {/each}
    </section>
  {/if}
  {#if c.skillPriority.length || c.spells.length}
    <section class="grid gap-2 sm:grid-cols-2" data-testid="lol-champion-skills">
      {#if c.skillPriority.length}
        <div>
          <h4 class="m-0 mb-1 text-xs font-medium text-foreground">{m.plugin_lol_skills()}</h4>
          <p class="m-0 text-xs text-foreground/90">{m.plugin_lol_skill_priority({ order: c.skillPriority.join(" > ") })}</p>
          {#if c.skillOrder.length}
            <div class="mt-1 flex flex-wrap gap-0.5" aria-label={m.plugin_lol_skill_order()}>
              {#each c.skillOrder as key, i (i)}
                <span class="grid size-5 place-items-center rounded text-[10px] font-medium {key === 'R' ? 'bg-primary/20 text-primary' : 'bg-muted text-foreground/80'}">{key}</span>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      {#if c.spells.length}
        <div>
          <h4 class="m-0 mb-1 text-xs font-medium text-foreground">{m.plugin_lol_spells()}</h4>
          <ul class="m-0 grid list-none gap-0.5 p-0 text-xs">
            {#each c.spells as pair, i (i)}
              <li class="flex gap-2"><span class="text-foreground">{names(pair.spells)}</span><span class="ml-auto text-[11px] text-muted-foreground tabular-nums">{rate(pair.pickRate)}</span></li>
            {/each}
          </ul>
        </div>
      {/if}
    </section>
  {/if}
  {#if c.opgg}
    <details class="rounded-md border border-hairline px-2 py-1.5 text-xs" data-testid="lol-champion-opgg">
      <summary class="flex cursor-pointer items-center gap-2 font-medium text-foreground">
        <span class="rounded bg-[#5383e8] px-1 text-[10px] font-bold text-white">OP.GG</span>{m.plugin_lol_opgg_title()}
      </summary>
      {#if c.opgg.text}
        <pre class="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap text-[11px] leading-snug text-foreground/85">{c.opgg.text}</pre>
      {:else if c.opgg.error}
        <p class="m-0 mt-1 text-destructive">{c.opgg.error}</p>
      {/if}
      <button type="button" class="mt-1 flex items-center gap-1 text-[11px] text-primary hover:underline" onclick={() => void openExternal(c.opgg?.url ?? "https://op.gg")}>
        <ExternalLink class="size-3" />{c.opgg.attribution}
      </button>
    </details>
  {/if}
{/snippet}

{#snippet mayhem(c: ChampionMayhem)}
  <p class="m-0 text-xs text-foreground/90" data-testid="lol-champion-mayhem-rank">
    {m.plugin_lol_mayhem_rank({ rank: c.rank ?? "—", win: rate(c.winRate), pick: rate(c.pickRate) })}
  </p>
  {#if c.partners.length}
    <div>
      <p class="m-0 mb-1 text-[11px] font-medium text-muted-foreground">{m.plugin_lol_partners()}</p>
      <div class="flex flex-wrap gap-1">
        {#each c.partners as p (p.id)}
          <button type="button" class="rounded-md bg-muted/50 px-2 py-0.5 text-xs hover:bg-muted" data-testid="lol-champion-link" onclick={() => void openChampion(p)}>{p.short}</button>
        {/each}
      </div>
    </div>
  {/if}
  {#if c.augments.length}
    <section aria-label={m.plugin_lol_best_augments()} data-testid="lol-champion-augments">
      <h4 class="m-0 mb-1 text-xs font-medium text-foreground">{m.plugin_lol_best_augments()}</h4>
      <p class="m-0 mb-1 text-[11px] text-muted-foreground">{m.plugin_lol_augments_note({ champion: c.champion.short })}</p>
      <ul class="m-0 grid list-none gap-1 p-0">
        {#each c.augments as a (a.id)}
          <li class="flex min-w-0 items-center gap-2 rounded-md bg-muted/40 px-2 py-1 text-xs {a.lowSample ? 'opacity-60' : ''}" data-testid="lol-champion-augment">
            <span class="size-2 shrink-0 rounded-full {rarityClass(a.rarity)}"></span>
            <span class="min-w-0 flex-1 truncate text-foreground">{a.name}</span>
            <span class="shrink-0 font-medium tabular-nums {a.delta >= 0 ? 'text-success' : 'text-destructive'}">{signed(a.delta)}</span>
            <span class="shrink-0 text-[11px] text-muted-foreground tabular-nums">{m.plugin_lol_augment_row({ win: rate(a.winRate), games: compact(a.games) })}</span>
            {#if a.lowSample}<span class="shrink-0 text-[10px] text-warning">{m.plugin_lol_low_sample()}</span>{/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
  {#if c.attribution}
    <p class="m-0 flex flex-wrap items-center gap-x-1 text-[11px] text-muted-foreground" data-testid="lol-champion-attribution">
      <span>{c.attribution}</span>
      <button type="button" class="flex items-center gap-0.5 text-primary hover:underline" onclick={() => void openExternal(ARAMHEX_URL)}><ExternalLink class="size-3" />{m.plugin_lol_attribution_open()}</button>
    </p>
  {/if}
{/snippet}

<div class="space-y-3" data-testid="lol-champion-page" data-mode={champion?.mode ?? ""}>
  <form class="flex items-center gap-1.5" onsubmit={search}>
    <Input bind:value={query} placeholder={m.plugin_lol_champion_search_placeholder()} class="h-8 text-xs" data-testid="lol-champion-search" />
    <Button type="submit" size="sm" variant="outline" class="h-8 gap-1 px-2.5 text-xs" disabled={loading || !query.trim()} data-testid="lol-champion-search-go">
      <Search class="size-3" />{m.plugin_lol_search()}
    </Button>
  </form>
  {#if champion}
    <div class="flex items-center gap-2">
      <div class="min-w-0 flex-1">
        <p class="m-0 truncate text-sm font-semibold text-foreground" data-testid="lol-champion-name">{champion.champion.label}</p>
        <p class="m-0 text-[11px] text-muted-foreground">
          {champion.mode === "rift" ? `${m.plugin_lol_mode_rift()} · ${champion.laneName} · ${champion.patch}` : m.plugin_lol_mode_mayhem()}
        </p>
      </div>
      <div class="flex shrink-0 rounded-lg bg-muted/70 p-0.5" role="group" aria-label={m.plugin_lol_mode()}>
        {#each [["rift", m.plugin_lol_mode_rift()], ["mayhem", m.plugin_lol_mode_mayhem()]] as [mode, label] (mode)}
          <button type="button" data-testid={`lol-champion-mode-${mode}`} aria-pressed={champion.mode === mode}
            class="h-6 rounded-md px-2 text-[11px] font-medium {champion.mode === mode ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            onclick={() => switchMode(mode as "rift" | "mayhem")}>{label}</button>
        {/each}
      </div>
      <Button size="icon" variant="outline" class="size-7 shrink-0" aria-label={m.plugin_lol_refresh()} title={m.plugin_lol_refresh()} data-testid="lol-refresh" disabled={loading} onclick={refresh}>
        {#if loading}<LoaderCircle class="size-3 animate-spin" />{:else}<RotateCw class="size-3" />{/if}
      </Button>
    </div>
  {/if}
  {#if snapshot}<p class="m-0 text-[11px] text-muted-foreground" data-testid="lol-snapshot">{m.plugin_lol_snapshot({ companion: $companionName })}</p>{/if}
  {#if error}<p role="alert" class="m-0 text-xs text-destructive">{m.plugin_lol_failed({ error })}</p>{/if}
  {#if status}<p class="m-0 rounded-lg bg-muted/50 px-3 py-2 text-xs text-foreground/85" data-testid="lol-status">{status}</p>{/if}
  {#if champion}
    {#if champion.mode === "rift"}{@render rift(champion)}{:else}{@render mayhem(champion)}{/if}
    <p class="m-0 border-t border-hairline pt-2 text-[11px] text-muted-foreground" data-testid="lol-champion-sources">
      {m.plugin_lol_sources({ sources: sourcesLine(champion.sources) })}{#if champion.mode === "rift" && champion.stale} · {m.plugin_lol_stale()}{/if}
    </p>
  {:else if loading}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_lol_loading()}</p>
  {:else if !status}
    <p class="m-0 text-xs text-muted-foreground" data-testid="lol-champion-empty">{m.plugin_lol_champion_empty()}</p>
  {/if}
</div>
