<script lang="ts">
  // 强度榜（`ui://kotone/lol/tier`，`lol.tier` 的页面）：峡谷按分路（强度、胜率、选用率、禁用率、克制它的），
  // 海斗是一张英雄榜；每行有英雄头像。数据来源写在最下面。
  // 样子先朴素：之后照 docs/design/lol-app.md 整个重做。
  import { onMount } from "svelte";
  import ChampionIcon from "../ChampionIcon.svelte";
  import { resultText, TOOL_TIER, type Session, type ToolResult } from "../host";
  import { t } from "../i18n.svelte";
  import { isTierData, rate, sourcesLine, type TierData } from "../types";

  let { session }: { session: Session } = $props();
  let tier = $state<TierData | null>(null);
  let lane = $state("");
  let status = $state("");
  let error = $state("");
  let loading = $state(true);

  function show(result: ToolResult) {
    if (isTierData(result.structuredContent)) {
      tier = result.structuredContent;
      status = "";
    } else {
      status = resultText(result);
    }
  }

  async function load(args: Record<string, unknown>) {
    loading = true;
    error = "";
    try {
      show(await session.call(TOOL_TIER, args));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    void session.initial.then((result) => {
      if (result) {
        show(result);
        loading = false;
      } else {
        void load({});
      }
    });
  });

  const lanes = $derived(tier?.mode === "rift" ? tier.laneNames : []);
  const shownLane = $derived(lanes.some((l) => l.id === lane) ? lane : (lanes[0]?.id ?? ""));
  const riftRows = $derived(tier?.mode === "rift" ? (tier.lanes[shownLane] ?? []) : []);
</script>

<main class="page" data-testid="lol-app-tier" data-mode={tier?.mode ?? ""}>
  <header class="bar">
    <h1>{t("plugin_lol_page_tier")}</h1>
    <div class="modes" role="group" aria-label={t("plugin_lol_mode")}>
      {#each [["rift", t("plugin_lol_mode_rift")], ["mayhem", t("plugin_lol_mode_mayhem")]] as [mode, label] (mode)}
        <button type="button" aria-pressed={tier?.mode === mode} data-testid={`lol-app-mode-${mode}`} onclick={() => void load({ mode })}>{label}</button>
      {/each}
    </div>
    <button type="button" class="refresh" disabled={loading} data-testid="lol-app-refresh" onclick={() => void load({ mode: tier?.mode ?? "rift" })}>
      {loading ? t("plugin_lol_loading") : t("plugin_lol_refresh")}
    </button>
  </header>

  {#if error}<p class="note error" role="alert">{t("plugin_lol_failed", { error })}</p>{/if}
  {#if status}<p class="note" data-testid="lol-app-status">{status}</p>{/if}

  {#if tier?.mode === "rift"}
    <div class="lanes" role="tablist" aria-label={t("plugin_lol_lane")}>
      {#each lanes as l (l.id)}
        <button type="button" role="tab" aria-selected={shownLane === l.id} onclick={() => (lane = l.id)}>{l.name}</button>
      {/each}
    </div>
    <ol class="rows" data-testid="lol-app-rows" data-lane={shownLane}>
      {#each riftRows as row (row.champion.id)}
        <li class="row">
          <span class="rank">{row.rank ?? ""}</span>
          <ChampionIcon id={row.champion.id} label={row.champion.short} />
          <span class="tier" data-tier={row.tier}>{row.tier}</span>
          <span class="name">{row.champion.short}</span>
          <span class="stats">{t("plugin_lol_tier_stats", { win: rate(row.winRate), pick: rate(row.pickRate), ban: rate(row.banRate) })}</span>
          {#if row.counters.length}
            <span class="counters">{t("plugin_lol_tier_counters", { names: row.counters.map((c) => c.short).join(" · ") })}</span>
          {/if}
        </li>
      {/each}
    </ol>
  {:else if tier?.mode === "mayhem"}
    <ol class="rows" data-testid="lol-app-rows" data-lane="mayhem">
      {#each tier.rows as row (row.champion.id)}
        <li class="row">
          <span class="rank">{row.rank ?? ""}</span>
          <ChampionIcon id={row.champion.id} label={row.champion.short} />
          <span class="name">{row.champion.short}</span>
          {#if row.rankChange}<span class="change" class:up={row.rankChange > 0}>{row.rankChange > 0 ? `↑${row.rankChange}` : `↓${-row.rankChange}`}</span>{/if}
          <span class="stats">{t("plugin_lol_win_pick", { win: rate(row.winRate), pick: rate(row.pickRate) })}</span>
        </li>
      {/each}
    </ol>
  {:else if loading}
    <p class="note">{t("plugin_lol_loading")}</p>
  {/if}

  {#if tier}
    <footer class="sources" data-testid="lol-app-sources">{t("plugin_lol_sources", { sources: sourcesLine(tier.sources) })}</footer>
  {/if}
</main>

<style>
  .page {
    display: grid;
    gap: 10px;
    padding: 12px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h1 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  button {
    font: inherit;
    font-size: 12px;
    color: inherit;
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 3px 9px;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: var(--surface-2);
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .modes,
  .lanes {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .modes button[aria-pressed="true"],
  .lanes button[aria-selected="true"] {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .rows {
    display: grid;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--surface);
  }
  .rank {
    width: 20px;
    text-align: right;
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .tier {
    padding: 0 5px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 700;
    background: var(--surface-2);
    color: var(--muted);
  }
  .tier[data-tier="T0"] {
    background: var(--hot-soft);
    color: var(--hot);
  }
  .tier[data-tier="T1"] {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .name {
    flex: 1;
    min-width: 4em;
    font-weight: 600;
    font-size: 13px;
  }
  .stats {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .counters {
    flex-basis: 100%;
    padding-left: 64px;
    font-size: 11px;
    color: var(--muted);
  }
  .change {
    font-size: 11px;
    color: var(--hot);
  }
  .change.up {
    color: var(--good);
  }
  .note {
    margin: 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--surface-2);
    font-size: 12px;
  }
  .note.error {
    color: var(--hot);
  }
  .sources {
    padding-top: 8px;
    border-top: 1px solid var(--line);
    font-size: 11px;
    color: var(--muted);
  }
</style>
