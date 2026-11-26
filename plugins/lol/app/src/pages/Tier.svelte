<script lang="ts">
  // 强度榜（`ui://kotone/lol/tier`，`lol.tier` 的页面；docs/design/lol-app.md 第三节 5）。
  // 全屏：模式（峡谷 | 海斗）+ 分路，一行 44 px：名次 · 级别 · 英雄 · 胜率 · 选取 · 禁用 · 被谁克制；海斗是名次 · 变化 · 英雄 · 胜率 · 选取。
  // 级别小牌用墨的深浅（T0 墨底反白、T1 墨框墨字、T2 以下次要字色），不用彩色。
  // 卡片（inline）：前 10 名，一行 28 px。
  import { onMount } from "svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import LaneIcon from "../components/LaneIcon.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { laneName, rate } from "../format";
  import { PAGE_SETTINGS, resultStatus, TOOL_TIER, type DisplayMode, type Session, type ToolResult } from "../host";
  import StateBlock from "../components/StateBlock.svelte";
  import type { ToolState } from "../status";
  import { m } from "../i18n.svelte";
  import { isTierData, type TierData } from "../types";


  let {
    session,
    initial,
    display,
    args = {},
    onTab,
    onChampion,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    /** `{ mode, lane }`。 */
    args?: Record<string, unknown>;
    onTab: (page: PageId) => void;
    /** 点一行：进这个英雄的页（同一种模式、同一路）。 */
    onChampion: (args: Record<string, string>) => void;
  } = $props();

  let tier = $state<TierData | null>(null);
  let lane = $state("");
  let status = $state<ToolState | null>(null);
  let error = $state("");
  let loading = $state(true);

  function show(result: ToolResult) {
    if (isTierData(result.structuredContent)) {
      tier = result.structuredContent;
      status = null;
    } else {
      status = resultStatus(result);
    }
  }

  /** `first`：宿主没推结果时取的第一份（推得晚的宿主推来了就用推来的，见 host.ts 的 firstCall）。 */
  async function load(args: Record<string, unknown>, first = false) {
    loading = true;
    error = "";
    try {
      show(await (first ? session.firstCall : session.call)(TOOL_TIER, args));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    const start = initial ?? Promise.resolve(null);
    void start.then((result) => {
      if (result) {
        show(result);
        loading = false;
      } else {
        const given = args.mode || args.lane ? args : session.input();
        void load(given && (given.mode || given.lane) ? { mode: given.mode, lane: given.lane } : {}, true);
      }
    });
  });

  const full = $derived(display !== "inline");
  const lanes = $derived(tier?.mode === "rift" ? tier.laneNames : []);
  const shownLane = $derived(lanes.some((l) => l.id === lane) ? lane : (lanes[0]?.id ?? ""));
  const riftRows = $derived(tier?.mode === "rift" ? (tier.lanes[shownLane] ?? []) : []);

  const tierTone = (t: string) => (t === "T0" ? "t0" : t === "T1" ? "t1" : "");

  /** 点一行进英雄页（同一种模式、同一路）。 */
  function openChampion(name: string) {
    if (!tier) return;
    const next: Record<string, string> = { champion: name, mode: tier.mode };
    if (tier.mode === "rift" && shownLane) next.lane = shownLane;
    onChampion(next);
  }
  function key(event: KeyboardEvent, name: string) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openChampion(name);
    }
  }
</script>

{#snippet tierBadge(t: string)}<span class="tier {tierTone(t)}" data-tier={t}>{t}</span>{/snippet}

<div class="app" data-testid="lol-app-tier" data-mode={tier?.mode ?? ""}>
  {#if full}
    <TopBar current="tier" {onTab} onRefresh={() => void load({ mode: tier?.mode ?? "rift" })} {loading} />
  {/if}
  <main class={full ? "page" : "card"}>
    {#if full}
      <div class="tools">
        <span class="seg" role="group" aria-label={m.plugin_lol_mode()}>
          {#each [["rift", m.plugin_lol_mode_rift()], ["mayhem", m.plugin_lol_mode_mayhem()]] as [mode, label] (mode)}
            <button type="button" aria-pressed={tier?.mode === mode} data-testid="lol-app-mode-{mode}" onclick={() => void load({ mode })}>{label}</button>
          {/each}
        </span>
        {#if tier?.mode === "rift"}
          <span class="lanes" role="tablist" aria-label={m.plugin_lol_lane()}>
            {#each lanes as l (l.id)}
              <button type="button" role="tab" class="lane" aria-selected={shownLane === l.id} onclick={() => (lane = l.id)}>
                <LaneIcon lane={l.id} />{laneName(l.id) || l.name}
              </button>
            {/each}
          </span>
        {/if}
      </div>
    {:else}
      <div class="card-head">
        <span class="t">{m.plugin_lol_page_tier()}</span>
        <span class="sub">{tier?.mode === "mayhem" ? m.plugin_lol_mode_mayhem() : [m.plugin_lol_mode_rift(), laneName(shownLane)].filter(Boolean).join(" · ")}</span>
        {#if session.context?.availableDisplayModes?.includes("fullscreen")}
          <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} onclick={() => void session.requestDisplayMode("fullscreen")}>
            <Icon name="open" />
          </button>
        {/if}
      </div>
    {/if}

    {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
    {#if status}<StateBlock state={status} inline={!full} onSettings={() => session.openPage(PAGE_SETTINGS, {})} />{/if}

    {#if tier?.mode === "rift"}
      <div class="table rift" data-testid="lol-app-rows" data-lane={shownLane}>
        <div class="row head">
          <span class="c-rank">#</span><span>{m.plugin_lol_col_tier()}</span><span>{m.plugin_lol_col_champion()}</span>
          <span class="r">{m.plugin_lol_col_win()}</span><span class="r">{m.plugin_lol_col_pick()}</span>
          {#if full}<span class="r wide">{m.plugin_lol_col_ban()}</span><span class="wide">{m.plugin_lol_col_counters()}</span>{/if}
        </div>
        {#each full ? riftRows : riftRows.slice(0, 10) as row (row.champion.id)}
          <div class="row link" role="link" tabindex="0" data-testid="lol-app-tier-row" title={row.champion.label}
            onclick={() => openChampion(row.champion.short)} onkeydown={(e) => key(e, row.champion.short)}>
            <span class="c-rank num">{row.rank ?? ""}</span>
            {@render tierBadge(row.tier)}
            <span class="champ"><ChampIcon id={row.champion.id} icon={row.champion.icon} name={row.champion.short} size={full ? 32 : 24} /><b>{row.champion.short}</b></span>
            <span class="r num">{rate(row.winRate)}</span>
            <span class="r num">{rate(row.pickRate)}</span>
            {#if full}
              <span class="r num wide">{rate(row.banRate)}</span>
              <span class="counters wide" title={m.plugin_lol_tier_counters({ names: row.counters.map((c) => c.short).join(" · ") })}>
                {#each row.counters as c (c.id)}<ChampIcon id={c.id} icon={c.icon} name={c.short} size={20} />{/each}
              </span>
            {/if}
          </div>
        {/each}
      </div>
    {:else if tier?.mode === "mayhem"}
      <div class="table mayhem" data-testid="lol-app-rows" data-lane="mayhem">
        <div class="row head">
          <span class="c-rank">#</span><span>{m.plugin_lol_col_change()}</span><span>{m.plugin_lol_col_champion()}</span>
          <span class="r">{m.plugin_lol_col_win()}</span><span class="r">{m.plugin_lol_col_pick()}</span>
        </div>
        {#each full ? tier.rows : tier.rows.slice(0, 10) as row (row.champion.id)}
          <div class="row link" role="link" tabindex="0" data-testid="lol-app-tier-row" title={row.champion.label}
            onclick={() => openChampion(row.champion.short)} onkeydown={(e) => key(e, row.champion.short)}>
            <span class="c-rank num">{row.rank ?? ""}</span>
            <span class="change num">
              {#if row.rankChange && row.rankChange > 0}<span class="good">↑{row.rankChange}</span>{:else if row.rankChange && row.rankChange < 0}<span class="bad">↓{-row.rankChange}</span>{/if}
            </span>
            <span class="champ"><ChampIcon id={row.champion.id} icon={row.champion.icon} name={row.champion.short} size={full ? 32 : 24} /><b>{row.champion.short}</b></span>
            <span class="r num">{rate(row.winRate)}</span>
            <span class="r num">{rate(row.pickRate)}</span>
          </div>
        {/each}
      </div>
    {:else if loading}
      <div class="table" aria-busy="true" aria-label={m.plugin_lol_loading()}>
        {#each [0, 1, 2, 3, 4, 5] as i (i)}<div class="row"><span class="skel" style:width="16px"></span><span class="ci s32"></span><span class="skel" style:width="{90 + i * 9}px"></span></div>{/each}
      </div>
    {/if}

    {#if tier}
      {#if full}
        <!-- 强度榜只有腾讯 101 一个来源（ADR 043 D8：标签只写出处） -->
        <span data-testid="lol-app-sources"><Footer sources={[{ id: "qq101", date: tier.date, stale: tier.stale }]} onLink={session.openLink} /></span>
      {:else}
        <span data-testid="lol-app-sources"><Footer card sources={[{ id: "qq101", date: tier.date, stale: tier.stale }]} onLink={session.openLink} /></span>
      {/if}
    {/if}
  </main>
</div>

<style>
  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
    margin-bottom: var(--lol-sp-5);
  }
  .lanes {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .lane {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid transparent;
    border-radius: var(--lol-r-md);
    background: none;
    cursor: pointer;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .lane:hover {
    color: var(--lol-text);
  }
  .lane[aria-selected="true"] {
    border-color: var(--lol-text);
    color: var(--lol-text);
    font-weight: var(--lol-fw-medium);
  }
  .table {
    overflow: hidden;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-lg);
  }
  .card .table {
    border: 0;
    border-radius: 0;
  }
  .row {
    display: grid;
    align-items: center;
    column-gap: 10px;
    min-height: var(--lol-row-list);
    padding: 0 12px;
  }
  .rift .row {
    grid-template-columns: 2em 2.6em minmax(8em, 1fr) 4.6em 4.6em 4.6em 6.5em;
  }
  .mayhem .row {
    grid-template-columns: 2em 2.6em minmax(8em, 1fr) 4.6em 4.6em;
  }
  .card .rift .row {
    grid-template-columns: 1.6em 2.4em minmax(0, 1fr) 4.4em 4.4em;
  }
  .card .row {
    min-height: var(--lol-row-card);
  }
  .table:not(.rift):not(.mayhem) .row {
    display: flex;
  }
  .row + .row {
    border-top: 1px solid var(--lol-line);
  }
  .row.link {
    cursor: pointer;
  }
  .row.link:hover {
    background: var(--lol-hover);
  }
  .row.head {
    min-height: 30px;
    background: var(--lol-bg-2);
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
  }
  .card .row.head {
    min-height: 24px;
    background: transparent;
  }
  .r {
    text-align: right;
  }
  .c-rank {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    text-align: right;
  }
  .champ {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .champ b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-strong);
  }
  .card .champ b {
    font-size: var(--lol-fs-sm);
  }
  .tier {
    justify-self: start;
    padding: 0 5px;
    border: 1px solid transparent;
    border-radius: var(--lol-r-xs);
    font-size: var(--lol-fs-xs);
    line-height: 16px;
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text-3);
  }
  .tier.t0 {
    background: var(--lol-ink-fill);
    color: var(--lol-ink-on);
  }
  .tier.t1 {
    border-color: var(--lol-text);
    color: var(--lol-text);
  }
  .counters {
    display: flex;
    gap: 2px;
  }
  .change {
    font-size: var(--lol-fs-xs);
  }
  @container (max-width: 519px) {
    .wide {
      display: none;
    }
    .rift .row {
      grid-template-columns: 1.6em 2.4em minmax(0, 1fr) 4.4em 4.4em;
    }
  }
</style>
