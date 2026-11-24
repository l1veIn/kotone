<script lang="ts">
  // 英雄（`ui://kotone/lol/champion`，`lol.champion` 的页面；docs/design/lol-app.md 第三节 4，调研 G39）。
  // 峡谷：头部（头像 56、名字、分路、版本、强度和胜率）、换分路、难打 / 好打（点英雄进它的页）、出装（核心、鞋、出门、
  // 第四到第六件）、符文、加点（主升和前 18 级）、召唤师技能、OP.GG（开了才有，折叠）。
  // 海斗：英雄榜名次和胜率、最搭的队友、最好的海克斯（比这个英雄平均胜率高几个百分点，样本少的变淡）。
  // 出处写在页脚：腾讯 101（版本 · 日期）、aramhex-data（CC BY-NC 4.0，署名、不商用）、OP.GG。
  // 卡片（inline，她调 `lol.champion` 时）：头像 + 强度一行；核心出装一行；海斗时前 3 个海克斯。
  import { onMount } from "svelte";
  import Augment from "../components/Augment.svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import LaneIcon from "../components/LaneIcon.svelte";
  import Pic from "../components/Pic.svelte";
  import StateBlock from "../components/StateBlock.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { laneName, num, rate } from "../format";
  import { PAGE_SETTINGS, resultStatus, TOOL_CHAMPION, type DisplayMode, type Session, type ToolResult } from "../host";
  import { m } from "../i18n.svelte";
  import type { ToolState } from "../status";
  import { ARAMHEX_URL, isChampionData, type BuildOption, type ChampionData, type Named, type Source } from "../types";

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
    /** `{ champion, mode, lane }`（强度榜、克制关系里点进来）。 */
    args?: Record<string, unknown>;
    onTab: (page: PageId) => void;
    /** 点另一个英雄（克制关系、搭档）：进它的页。 */
    onChampion: (args: Record<string, string>) => void;
  } = $props();

  let data = $state<ChampionData | null>(null);
  let status = $state<ToolState | null>(null);
  let error = $state("");
  let loading = $state(false);
  let query = $state("");
  let mode = $state<"rift" | "mayhem">("rift");
  let lastArgs = $state<Record<string, unknown> | null>(null);

  const full = $derived(display !== "inline");
  const canOpen = $derived(!!session.context?.availableDisplayModes?.includes("fullscreen"));
  const LANES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "SUPPORT"];

  function show(result: ToolResult) {
    if (isChampionData(result.structuredContent)) {
      data = result.structuredContent;
      mode = data.mode;
      status = null;
    } else {
      status = resultStatus(result);
    }
  }

  async function load(next: Record<string, unknown>) {
    lastArgs = next;
    loading = true;
    error = "";
    try {
      show(await session.call(TOOL_CHAMPION, next));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function fromArgs(a: Record<string, unknown> | null): Record<string, unknown> | null {
    if (!a || typeof a.champion !== "string" || !a.champion) return null;
    const out: Record<string, unknown> = { champion: a.champion };
    if (a.mode === "rift" || a.mode === "mayhem") out.mode = a.mode;
    if (typeof a.lane === "string" && a.lane) out.lane = a.lane;
    return out;
  }

  onMount(() => {
    session.onResult(show);
    if (args.mode === "mayhem") mode = "mayhem";
    const own = fromArgs(args);
    if (initial) void initial.then((result) => {
      if (result) show(result);
      else {
        const given = own ?? fromArgs(session.input());
        if (given) void load(given);
      }
    });
    else if (own) void load(own);
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    const q = query.trim();
    if (q) void load({ champion: q, mode });
  }
  function switchMode(next: "rift" | "mayhem") {
    mode = next;
    const name = data?.champion.short ?? query.trim();
    if (name) void load({ champion: name, mode: next });
  }
  function switchLane(lane: string) {
    if (data) void load({ champion: data.champion.short, mode: "rift", lane });
  }
  function openOther(name: string) {
    const next: Record<string, string> = { champion: name, mode: data?.mode ?? mode };
    if (data?.mode === "rift" && data.lane) next.lane = data.lane;
    onChampion(next);
  }
  function key(event: KeyboardEvent, name: string) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openOther(name);
    }
  }

  /** “腾讯 101（16.23 · 20261124）”。 */
  function sourcesLine(sources: Source[], extra = ""): string {
    return sources
      .map((s) => {
        const more = [s.license ?? "", s.date ? s.date.slice(0, 10) : ""].filter(Boolean).join(" · ");
        return more ? m.plugin_lol_source_with({ name: s.name, extra: more }) : s.name;
      })
      .concat(extra ? [extra] : [])
      .join(" · ");
  }
  const footData = $derived(
    data
      ? m.plugin_lol_sources({
          sources:
            sourcesLine(data.sources, data.mode === "rift" && data.opgg ? "OP.GG" : "") +
            (data.mode === "rift" && data.stale ? ` · ${m.plugin_lol_stale()}` : ""),
        })
      : m.plugin_lol_sources({ sources: "—" }),
  );
  const BUILD_GROUPS = [
    ["core", () => m.plugin_lol_build_core()],
    ["boots", () => m.plugin_lol_build_boots()],
    ["starting", () => m.plugin_lol_build_starting()],
    ["fourth", () => m.plugin_lol_build_fourth()],
    ["fifth", () => m.plugin_lol_build_fifth()],
    ["sixth", () => m.plugin_lol_build_sixth()],
  ] as const;
  const optionRate = (o: BuildOption) => m.plugin_lol_win_pick({ win: rate(o.winRate), pick: rate(o.pickRate) });
  const KEYS = ["Q", "W", "E", "R"];
</script>

{#snippet item(x: Named, size: number)}
  <Pic path={x.icon} name={x.name || String(x.id)} {size} />
{/snippet}

{#snippet keyTile(k: string, size: number)}
  <span class="key s{size}" class:ult={k === "R"} title={k}>{k}</span>
{/snippet}

{#if full}
  <div class="app" data-testid="lol-app-champion" data-mode={data?.mode ?? ""}>
    <TopBar current="champion" {onTab} onRefresh={lastArgs ? () => void load(lastArgs ?? {}) : undefined} {loading} />
    <main class="page">
      <div class="tools">
        <form class="search" onsubmit={search}>
          <span class="field"><Icon name="search" /><input bind:value={query} placeholder={m.plugin_lol_champion_search_placeholder()}
            aria-label={m.plugin_lol_champion_search_placeholder()} data-testid="lol-champion-search" /></span>
          <button type="submit" class="btn primary" disabled={loading} data-testid="lol-champion-search-go">{m.plugin_lol_search()}</button>
        </form>
        <span class="seg" role="group" aria-label={m.plugin_lol_mode()}>
          <button type="button" aria-pressed={mode === "rift"} data-testid="lol-champion-mode-rift" onclick={() => switchMode("rift")}>{m.plugin_lol_mode_rift()}</button>
          <button type="button" aria-pressed={mode === "mayhem"} data-testid="lol-champion-mode-mayhem" onclick={() => switchMode("mayhem")}>{m.plugin_lol_mode_mayhem()}</button>
        </span>
      </div>
      {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
      {#if status}
        <StateBlock state={status} onSettings={() => session.openPage(PAGE_SETTINGS, {})} />
      {:else if !data && !loading}
        <p class="hint" data-testid="lol-champion-empty">{m.plugin_lol_champion_empty()}</p>
      {/if}

      {#if data?.mode === "rift"}
        {@const d = data}
        <header class="head">
          <ChampIcon id={d.champion.id} icon={d.champion.icon} name={d.champion.short} size={56} />
          <div>
            <p class="name" data-testid="lol-champion-name">{d.champion.label}</p>
            <p class="meta num">
              <span class="lane-now"><LaneIcon lane={d.lane} />{laneName(d.lane) || d.laneName}</span>
              <span>{d.patch}</span>
              {#if d.tier}
                <span class="tier {d.tier.tier === 'T0' ? 't0' : d.tier.tier === 'T1' ? 't1' : ''}" data-testid="lol-champion-tier">{d.tier.tier}</span>
                <span>{m.plugin_lol_tier_stats({ win: rate(d.tier.winRate), pick: rate(d.tier.pickRate), ban: rate(d.tier.banRate) })}</span>
              {/if}
            </p>
          </div>
        </header>
        <div class="lanes" role="group" aria-label={m.plugin_lol_lane()}>
          <span class="k">{m.plugin_lol_lane()}</span>
          {#each LANES as l (l)}
            <button type="button" class="lane" aria-pressed={d.lane === l} data-testid="lol-champion-lane-{l}" onclick={() => switchLane(l)}><LaneIcon lane={l} />{laneName(l)}</button>
          {/each}
        </div>
        <div class="vs">
          {#each [["bad", m.plugin_lol_bad_against(), d.badAgainst], ["good", m.plugin_lol_good_against(), d.goodAgainst]] as const as [id, label, list] (id)}
            <section class="box" data-testid="lol-champion-{id}">
              <h3>{label}</h3>
              {#each list.slice(0, 6) as x (x.champion.id)}
                <div class="vs-row" role="link" tabindex="0" data-testid="lol-champion-link" onclick={() => openOther(x.champion.short)} onkeydown={(e) => key(e, x.champion.short)}>
                  <ChampIcon id={x.champion.id} icon={x.champion.icon} name={x.champion.short} size={24} /><span class="grow">{x.champion.short}</span>
                  <span class="num">{rate(x.winRate)}</span>
                </div>
              {:else}<p class="muted">—</p>{/each}
            </section>
          {/each}
        </div>
        <section class="sec" data-testid="lol-champion-build">
          <h3>{m.plugin_lol_build()}</h3>
          {#each BUILD_GROUPS as [group, label] (group)}
            {@const options = d.build[group]}
            {#if options.length}
              <div class="build-row" data-testid={group === "core" ? "lol-champion-core" : undefined}>
                <span class="k">{label()}</span>
                <span class="opts">
                  {#each options.slice(0, group === "core" ? 2 : 1) as o, i (i)}
                    <span class="opt">
                      <span class="icons">{#each o.items as x, j (j)}{#if j > 0 && group === "core"}<span class="arrow">›</span>{/if}{@render item(x, 28)}{/each}</span>
                      <span class="names">{o.items.map((x) => x.name).join(" · ")}</span>
                      <span class="num muted">{optionRate(o)}</span>
                    </span>
                  {/each}
                </span>
              </div>
            {/if}
          {/each}
        </section>
        {#if d.runes.length}
          <section class="sec" data-testid="lol-champion-runes">
            <h3>{m.plugin_lol_runes()}</h3>
            {#each d.runes.slice(0, 2) as r, i (i)}
              <div class="rune-row">
                <span class="runes">
                  {#each r.primary as x, j (j)}<Pic path={x.icon} name={x.name || String(x.id)} size={j === 0 ? 32 : 24} round cls="rune" />{/each}
                  <span class="bar-sep"></span>
                  {#each r.secondary as x, j (j)}<Pic path={x.icon} name={x.name || String(x.id)} size={24} round cls="rune" />{/each}
                  <span class="bar-sep"></span>
                  {#each r.shards as x, j (j)}<Pic path={x.icon} name={x.name || String(x.id)} size={16} round cls="rune" />{/each}
                </span>
                <span class="names">{r.primary.map((x) => x.name).filter(Boolean).slice(0, 1).join("")}</span>
                <span class="num muted">{m.plugin_lol_win_pick({ win: rate(r.winRate), pick: rate(r.pickRate) })} · {m.plugin_lol_games_count({ games: num(r.games) })}</span>
              </div>
            {/each}
          </section>
        {/if}
        {#if d.skillPriority.length || d.skillOrder.length}
          <section class="sec" data-testid="lol-champion-skills">
            <h3>{m.plugin_lol_skills()}</h3>
            {#if d.skillPriority.length}
              <p class="prio"><span>{m.plugin_lol_skill_priority({ order: d.skillPriority.join(" > ") })}</span>
                <span class="keys">{#each KEYS as k (k)}{@render keyTile(k, 24)}{/each}</span></p>
            {/if}
            {#if d.skillOrder.length}
              <p class="order" aria-label={m.plugin_lol_skill_order()}>{#each d.skillOrder.slice(0, 18) as k, i (i)}{@render keyTile(k, 20)}{/each}</p>
            {/if}
          </section>
        {/if}
        {#if d.spells.length}
          <section class="sec">
            <h3>{m.plugin_lol_spells()}</h3>
            {#each d.spells.slice(0, 2) as p, i (i)}
              <p class="spell-row"><span class="icons">{#each p.spells as x, j (j)}{@render item(x, 24)}{/each}</span>
                <span class="names">{p.spells.map((x) => x.name).join(" + ")}</span>
                <span class="num muted">{m.plugin_lol_win_pick({ win: rate(p.winRate), pick: rate(p.pickRate) })}</span></p>
            {/each}
          </section>
        {/if}
        {#if d.opgg}
          <details class="sec opgg" data-testid="lol-champion-opgg">
            <summary><span class="chip">OP.GG</span>{m.plugin_lol_opgg_title()}</summary>
            {#if d.opgg.error}<p class="muted">{d.opgg.error}</p>{:else}<pre>{d.opgg.text}</pre>{/if}
            <p class="muted"><a href={d.opgg.url} onclick={(e) => { e.preventDefault(); session.openLink(d.opgg?.url ?? ""); }}>{d.opgg.attribution}</a></p>
          </details>
        {/if}
      {:else if data?.mode === "mayhem"}
        {@const d = data}
        <header class="head">
          <ChampIcon id={d.champion.id} icon={d.champion.icon} name={d.champion.short} size={56} />
          <div>
            <p class="name" data-testid="lol-champion-name">{d.champion.label}</p>
            <p class="meta num" data-testid="lol-champion-mayhem-rank">{m.plugin_lol_mayhem_rank({ rank: d.rank ?? "—", win: rate(d.winRate), pick: rate(d.pickRate) })}</p>
          </div>
        </header>
        {#if d.partners.length}
          <section class="sec">
            <h3>{m.plugin_lol_partners()}</h3>
            <p class="partners">
              {#each d.partners as c (c.id)}
                <span class="partner" role="link" tabindex="0" onclick={() => openOther(c.short)} onkeydown={(e) => key(e, c.short)}>
                  <ChampIcon id={c.id} icon={c.icon} name={c.short} size={24} />{c.short}</span>
              {/each}
            </p>
          </section>
        {/if}
        <section class="sec" data-testid="lol-champion-augments">
          <h3>{m.plugin_lol_best_augments()}</h3>
          <p class="muted note-line">{m.plugin_lol_augments_note({ champion: d.champion.short })}</p>
          {#each d.augments as a (a.id)}
            <div class="aug" class:low={a.lowSample} data-testid="lol-champion-augment">
              <Augment augment={a} size={40} />
              <span class="grow">{a.name}</span>
              <span class="delta num {a.delta > 0 ? 'good' : a.delta < 0 ? 'bad' : ''}">{a.delta > 0 ? "+" : ""}{a.delta.toFixed(1)}</span>
              <span class="num muted">{m.plugin_lol_augment_row({ win: rate(a.winRate), games: num(a.games) })}</span>
              {#if a.lowSample}<span class="chip">{m.plugin_lol_low_sample()}</span>{/if}
            </div>
          {:else}<p class="muted">—</p>{/each}
          {#if d.attribution}
            <p class="muted note-line" data-testid="lol-champion-attribution">{d.attribution} · <a href={ARAMHEX_URL} onclick={(e) => { e.preventDefault(); session.openLink(ARAMHEX_URL); }}>{m.plugin_lol_attribution_open()}</a></p>
          {/if}
        </section>
      {/if}
      {#if data}
        <span data-testid="lol-champion-sources"><Footer data={footData} onLink={session.openLink} /></span>
      {/if}
    </main>
  </div>
{:else}
  <div class="app card" data-testid="lol-app-champion" data-mode={data?.mode ?? ""}>
    {#if data}
      <div class="card-head">
        <ChampIcon id={data.champion.id} icon={data.champion.icon} name={data.champion.short} size={32} />
        <span class="t" data-testid="lol-champion-card-name">{data.champion.label}</span>
        <span class="sub num">
          {#if data.mode === "rift"}
            {laneName(data.lane) || data.laneName}{#if data.tier} · {data.tier.tier} · {m.plugin_lol_win_pick({ win: rate(data.tier.winRate), pick: rate(data.tier.pickRate) })}{/if}
          {:else}
            {m.plugin_lol_mayhem_rank({ rank: data.rank ?? "—", win: rate(data.winRate), pick: rate(data.pickRate) })}
          {/if}
        </span>
        {#if canOpen}
          <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} data-testid="lol-card-open"
            onclick={() => void session.requestDisplayMode("fullscreen")}><Icon name="open" /></button>
        {/if}
      </div>
      {#if data.mode === "rift"}
        {#if data.build.core[0]}
          <p class="card-row"><span class="k">{m.plugin_lol_build_core()}</span><span class="icons">{#each data.build.core[0].items as x, j (j)}{@render item(x, 24)}{/each}</span>
            <span class="names">{data.build.core[0].items.map((x) => x.name).join(" · ")}</span></p>
        {/if}
        {#if data.badAgainst.length}
          <p class="card-row"><span class="k">{m.plugin_lol_bad_against()}</span>
            {#each data.badAgainst.slice(0, 3) as x (x.champion.id)}<span class="mini-champ"><ChampIcon id={x.champion.id} icon={x.champion.icon} name={x.champion.short} size={20} />{x.champion.short}</span>{/each}</p>
        {/if}
      {:else}
        {#each data.augments.slice(0, 3) as a (a.id)}
          <p class="card-row"><Augment augment={a} size={20} /><span class="names">{a.name}</span>
            <span class="num {a.delta > 0 ? 'good' : ''}">{a.delta > 0 ? "+" : ""}{a.delta.toFixed(1)}</span></p>
        {/each}
      {/if}
      <Footer data={m.plugin_lol_sources({ sources: sourcesLine(data.sources) })} card onLink={session.openLink} />
    {:else if error}
      <p class="card-line danger" role="alert">{m.plugin_lol_failed({ error })}</p>
    {:else if status}
      <StateBlock state={status} inline />
    {:else}
      <p class="card-line" aria-busy="true">{m.plugin_lol_loading()}</p>
    {/if}
  </div>
{/if}

<style>
  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
    margin-bottom: var(--lol-sp-5);
  }
  .search {
    display: flex;
    flex: 1;
    gap: 8px;
    min-width: 240px;
  }
  .field {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--lol-line-2);
    border-radius: var(--lol-r-md);
    background: var(--lol-bg-2);
    color: var(--lol-text-3);
  }
  .field input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    font: inherit;
    color: var(--lol-text);
  }
  .search .btn {
    height: 32px;
  }
  .hint {
    margin: 40px 0;
    text-align: center;
    font-size: var(--lol-fs-md);
    color: var(--lol-text-2);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: var(--lol-sp-5);
  }
  .head p {
    margin: 0;
  }
  .name {
    font-size: var(--lol-fs-lg);
    line-height: var(--lol-lh-lg);
    font-weight: var(--lol-fw-strong);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 10px;
    color: var(--lol-text-2);
  }
  .lane-now {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .tier {
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
  .lanes {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-bottom: var(--lol-sp-5);
  }
  .k {
    flex: none;
    min-width: 4.5em;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
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
  .lane[aria-pressed="true"] {
    border-color: var(--lol-text);
    color: var(--lol-text);
    font-weight: var(--lol-fw-medium);
  }
  .vs {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--lol-sp-5);
    margin-bottom: var(--lol-sp-5);
  }
  @container (max-width: 559px) {
    .vs {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .box {
    overflow: hidden;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-lg);
  }
  .box h3,
  .sec h3 {
    margin: 0;
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text-3);
  }
  .box h3 {
    padding: 6px 12px;
    border-bottom: 1px solid var(--lol-line);
    background: var(--lol-bg-2);
  }
  .box .muted {
    margin: 8px 12px;
  }
  .vs-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 34px;
    padding: 0 12px;
    cursor: pointer;
  }
  .vs-row + .vs-row {
    border-top: 1px solid var(--lol-line);
  }
  .vs-row:hover,
  .partner:hover {
    background: var(--lol-hover);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .sec {
    display: grid;
    gap: 6px;
    margin-bottom: var(--lol-sp-5);
  }
  .build-row,
  .rune-row,
  .spell-row,
  .prio {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    margin: 0;
  }
  .opts {
    display: grid;
    flex: 1;
    gap: 6px;
    min-width: 0;
  }
  .opt {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 10px;
  }
  .icons {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 3px;
  }
  .arrow {
    color: var(--lol-text-3);
  }
  .names {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--lol-text-2);
  }
  .muted {
    color: var(--lol-text-3);
  }
  .runes {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .runes :global(.rune) {
    background: var(--lol-bg-3);
  }
  .bar-sep {
    width: 1px;
    height: 20px;
    margin: 0 4px;
    background: var(--lol-line);
  }
  .keys,
  .order {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 3px;
    margin: 0;
  }
  .key {
    display: inline-grid;
    place-items: center;
    width: var(--s);
    height: var(--s);
    border-radius: var(--lol-r-xs);
    background: var(--lol-bg-3);
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text-2);
  }
  .key.s24 {
    --s: 24px;
  }
  .key.s20 {
    --s: 20px;
  }
  .key.ult {
    background: var(--lol-ink-fill);
    color: var(--lol-ink-on);
  }
  .partners {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    margin: 0;
  }
  .partner {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    border-radius: var(--lol-r-md);
    cursor: pointer;
  }
  .note-line {
    margin: 0;
    font-size: var(--lol-fs-xs);
  }
  .note-line a,
  .opgg a {
    color: inherit;
  }
  .aug {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    padding: 0 4px;
  }
  .aug + .aug {
    border-top: 1px solid var(--lol-line);
  }
  .aug.low {
    opacity: 0.6;
  }
  .delta {
    width: 3.5em;
    text-align: right;
    font-weight: var(--lol-fw-medium);
  }
  .opgg summary {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .opgg pre {
    max-height: 240px;
    margin: 0;
    padding: 8px;
    overflow: auto;
    border-radius: var(--lol-r-md);
    background: var(--lol-bg-2);
    font-size: var(--lol-fs-xs);
    white-space: pre-wrap;
  }
  /* 卡片 */
  .card .card-head .t {
    flex: none;
  }
  .card-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 4px 12px;
    font-size: var(--lol-fs-xs);
  }
  .card-row .k {
    min-width: 3.5em;
  }
  .mini-champ {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
