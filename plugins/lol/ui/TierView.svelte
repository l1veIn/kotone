<script lang="ts">
  // 强度榜（`ui://kotone/lol/tier`，`lol.tier` 的页面）：峡谷按分路一个标签（强度、胜率、选用率、禁用率、克制它的），
  // 海斗是一张英雄榜。点一个英雄进它的英雄页（同一种模式）。数据来源写在最下面。
  import { onMount } from "svelte";
  import { LoaderCircle, RotateCw } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { FROM_PAGE, type PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_CHAMPION, statusText, type StatChampion, type TierData } from "./types";
  import { rate } from "./format";
  import { sourcesLine } from "./stats";

  let { params, data, bridge }: PluginViewProps = $props();
  const initial = () => (data as TierData | null) ?? null;
  let tier = $state<TierData | null>(initial());
  let lane = $state("");
  let status = $state("");
  let error = $state("");
  let loading = $state(false);

  async function load(args: Record<string, unknown>) {
    loading = true;
    error = "";
    try {
      const result = await bridge.call("lol.tier", args);
      if (result.structuredContent) {
        tier = result.structuredContent as TierData;
        status = "";
        bridge.replace({ ...args, [FROM_PAGE]: true }, tier);
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
    const { [FROM_PAGE]: _, ...args } = params;
    if (!tier) void load(args);
  });

  const lanes = $derived(tier?.mode === "rift" ? tier.laneNames : []);
  const shownLane = $derived(tier?.mode === "rift" ? (lanes.some((l) => l.id === lane) ? lane : (lanes[0]?.id ?? "")) : "");
  const rows = $derived(tier?.mode === "rift" ? (tier.lanes[shownLane] ?? []) : []);

  async function openChampion(c: StatChampion) {
    const mode = tier?.mode ?? "rift";
    const args: Record<string, unknown> = mode === "rift" ? { champion: c.short, mode, lane: shownLane } : { champion: c.short, mode };
    try {
      const result = await bridge.call("lol.champion", args);
      if (result.structuredContent) bridge.openPage(PAGE_CHAMPION, { ...args, [FROM_PAGE]: true }, result.structuredContent);
      else status = statusText(result.content);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  const tierClass = (t: string) => (t === "T0" ? "bg-destructive/15 text-destructive" : t === "T1" ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground");
</script>

<div class="space-y-3" data-testid="lol-tier-page" data-mode={tier?.mode ?? ""}>
  <div class="flex items-center gap-2">
    <div class="flex min-w-0 flex-1 rounded-lg bg-muted/70 p-0.5" role="group" aria-label={m.plugin_lol_mode()}>
      {#each [["rift", m.plugin_lol_mode_rift()], ["mayhem", m.plugin_lol_mode_mayhem()]] as [mode, label] (mode)}
        <button type="button" data-testid={`lol-tier-mode-${mode}`} aria-pressed={tier?.mode === mode}
          class="h-6 rounded-md px-2.5 text-[11px] font-medium {tier?.mode === mode ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => void load({ mode })}>{label}</button>
      {/each}
    </div>
    <Button size="icon" variant="outline" class="size-7 shrink-0" aria-label={m.plugin_lol_refresh()} title={m.plugin_lol_refresh()} data-testid="lol-refresh" disabled={loading}
      onclick={() => void load({ mode: tier?.mode ?? "rift" })}>
      {#if loading}<LoaderCircle class="size-3 animate-spin" />{:else}<RotateCw class="size-3" />{/if}
    </Button>
  </div>
  {#if error}<p role="alert" class="m-0 text-xs text-destructive">{m.plugin_lol_failed({ error })}</p>{/if}
  {#if status}<p class="m-0 rounded-lg bg-muted/50 px-3 py-2 text-xs text-foreground/85" data-testid="lol-status">{status}</p>{/if}
  {#if tier?.mode === "rift"}
    <div class="flex flex-wrap gap-1" role="tablist" aria-label={m.plugin_lol_lane()}>
      {#each lanes as l (l.id)}
        <button type="button" role="tab" aria-selected={shownLane === l.id} data-testid={`lol-tier-lane-${l.id}`}
          class="h-6 rounded-md px-2 text-[11px] {shownLane === l.id ? 'bg-primary/15 font-medium text-primary' : 'text-muted-foreground hover:bg-muted/60 hover:text-foreground'}"
          onclick={() => (lane = l.id)}>{l.name}</button>
      {/each}
    </div>
    <ul class="m-0 grid list-none gap-1 p-0" data-testid="lol-tier-rows" data-lane={shownLane}>
      {#each rows as row (row.champion.id)}
        <li>
          <button type="button" class="flex w-full min-w-0 items-center gap-2 rounded-md border border-hairline px-2 py-1.5 text-left text-xs hover:border-primary/40 hover:bg-muted/40"
            data-testid="lol-tier-row" onclick={() => void openChampion(row.champion)}>
            <span class="w-5 shrink-0 text-right text-[11px] text-muted-foreground tabular-nums">{row.rank ?? ""}</span>
            <span class="shrink-0 rounded px-1 text-[10px] font-semibold {tierClass(row.tier)}">{row.tier}</span>
            <span class="min-w-0 flex-1 truncate font-medium text-foreground">{row.champion.short}</span>
            <span class="shrink-0 text-[11px] text-foreground/85 tabular-nums">{m.plugin_lol_tier_stats({ win: rate(row.winRate), pick: rate(row.pickRate), ban: rate(row.banRate) })}</span>
          </button>
          {#if row.counters.length}
            <p class="m-0 truncate pl-9 text-[10px] text-muted-foreground">{m.plugin_lol_tier_counters({ names: row.counters.map((c) => c.short).join(" · ") })}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if tier?.mode === "mayhem"}
    <ul class="m-0 grid list-none gap-1 p-0" data-testid="lol-tier-rows" data-lane="mayhem">
      {#each tier.rows as row (row.champion.id)}
        <li>
          <button type="button" class="flex w-full min-w-0 items-center gap-2 rounded-md border border-hairline px-2 py-1.5 text-left text-xs hover:border-primary/40 hover:bg-muted/40"
            data-testid="lol-tier-row" onclick={() => void openChampion(row.champion)}>
            <span class="w-5 shrink-0 text-right text-[11px] text-muted-foreground tabular-nums">{row.rank ?? ""}</span>
            <span class="min-w-0 flex-1 truncate font-medium text-foreground">{row.champion.short}</span>
            {#if row.rankChange}<span class="shrink-0 text-[10px] tabular-nums {row.rankChange > 0 ? 'text-success' : 'text-destructive'}">{row.rankChange > 0 ? `↑${row.rankChange}` : `↓${-row.rankChange}`}</span>{/if}
            <span class="shrink-0 text-[11px] text-foreground/85 tabular-nums">{m.plugin_lol_win_pick({ win: rate(row.winRate), pick: rate(row.pickRate) })}</span>
          </button>
        </li>
      {/each}
    </ul>
  {:else if loading}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_lol_loading()}</p>
  {/if}
  {#if tier}
    <p class="m-0 border-t border-hairline pt-2 text-[11px] text-muted-foreground" data-testid="lol-tier-sources">{m.plugin_lol_sources({ sources: sourcesLine(tier.sources) })}</p>
  {/if}
</div>
