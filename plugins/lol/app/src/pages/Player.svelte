<script lang="ts">
  // 战绩（`ui://kotone/lol/player`，`lol.player` 的页面）。第一段先做能用的样子：搜人、头部（召唤师头像、段位）、
  // 近 20 局、每局一行（结果格、英雄、技能、符文、KDA、伤害、装备、海克斯）；点一局看单局详情。
  // 第二段照 docs/design/lol-app.md 第三节 2 做完整（常玩英雄、窄屏两行、卡片）。
  import { onMount } from "svelte";
  import Augment from "../components/Augment.svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import Items from "../components/Items.svelte";
  import Perks from "../components/Perks.svelte";
  import Pic from "../components/Pic.svelte";
  import RankBadge from "../components/RankBadge.svelte";
  import Spells from "../components/Spells.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { profileIconPath } from "../assets";
  import { duration, kdaOf, kdaTone, num, one, pctOf, rateTone, splitRiotId, when } from "../format";
  import { resultStatus, TOOL_PLAYER, type DisplayMode, type Session, type ToolResult } from "../host";
  import { m } from "../i18n.svelte";
  import { isPlayerData, type PlayerData } from "../types";

  let {
    session,
    initial,
    display,
    puuid = "",
    onTab,
    onGame,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    puuid?: string;
    onTab: (page: PageId) => void;
    onGame: (gameId: number) => void;
  } = $props();

  let player = $state<PlayerData | null>(null);
  let status = $state("");
  let error = $state("");
  let loading = $state(false);
  let query = $state("");
  let lastArgs: Record<string, unknown> = {};

  function show(result: ToolResult) {
    if (isPlayerData(result.structuredContent)) {
      player = result.structuredContent;
      status = "";
    } else {
      status = resultStatus(result).text;
    }
  }

  async function load(args: Record<string, unknown>) {
    lastArgs = args;
    loading = true;
    error = "";
    try {
      show(await session.call(TOOL_PLAYER, args));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    const given = session.input();
    const args = puuid ? { puuid } : initial && given ? given : {};
    if (initial) void initial.then((result) => (result ? show(result) : load(args)));
    else void load(args);
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    const q = query.trim();
    void load(q ? { player: q } : {});
  }

  const s = $derived(player?.summary && player.summary.games > 0 ? player.summary : null);
  const [nm, tag] = $derived(splitRiotId(player?.riotId ?? ""));
</script>

<div class="app" data-testid="lol-app-player">
  {#if display !== "inline"}
    <TopBar current="player" {onTab} onRefresh={() => void load(lastArgs)} {loading} />
  {/if}
  <main class="page">
    {#if display !== "inline"}
      <form class="search" onsubmit={search}>
        <span class="field"><Icon name="search" /><input bind:value={query} placeholder={m.plugin_lol_search_placeholder()} data-testid="lol-player-search" /></span>
        <button type="submit" class="btn primary" disabled={loading}>{m.plugin_lol_search()}</button>
      </form>
    {/if}
    {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
    {#if status}<p class="banner" data-testid="lol-app-status"><Icon name="info" />{status}</p>{/if}
    {#if player}
      <header class="who">
        <span class="avatar">
          <Pic path={player.profileIcon || profileIconPath(player.profileIconId)} name={nm || "?"} size={56} round />
          {#if player.summonerLevel}<span class="lvl num">{m.plugin_lol_level_short({ level: player.summonerLevel })}</span>{/if}
        </span>
        <div class="who-text">
          <p class="name">{nm}{#if tag}<span class="tagline">#{tag}</span>{/if}
            {#if player.hidden}<span class="chip warn"><Icon name="eyeOff" />{m.plugin_lol_hidden()}</span>{/if}
          </p>
          <p class="ranks">
            {#if player.ranked?.solo}<span>{m.plugin_lol_queue_solo()} <RankBadge ranked={player.ranked.solo} /></span>{/if}
            {#if player.ranked?.flex}<span>{m.plugin_lol_queue_flex()} <RankBadge ranked={player.ranked.flex} /></span>{/if}
            {#if !player.ranked && player.rank}<span>{player.rank}</span>{/if}
          </p>
          {#if s}
            <p class="sum num">
              {m.plugin_lol_recent_games({ count: s.games })}
              <b>{m.plugin_lol_wins_losses({ wins: s.wins, losses: s.losses })}</b>
              <b class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</b>
              <span>KDA <b class={kdaTone(s.kda)}>{one(s.kda)}</b></span>
              <span>{m.plugin_lol_avg_deaths()} <b>{one(s.deaths)}</b></span>
            </p>
          {/if}
        </div>
      </header>
      <ol class="games">
        {#each player.games as g (g.gameId)}
          {@const k = kdaOf(g.kills, g.deaths, g.assists)}
          <li>
            <button type="button" class="game" onclick={() => onGame(g.gameId)} data-testid="lol-player-game">
              <span class="res {g.win ? 'win' : 'loss'}"><b>{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</b><span class="num">{duration(g.durationS)}</span></span>
              <ChampIcon id={g.champion.id} icon={g.champion.icon} name={g.champion.short} level={g.level} />
              <Spells spells={g.spells} />
              <Perks perks={g.perks} />
              <span class="kda num"><b>{g.kills} / <span class="bad">{g.deaths}</span> / {g.assists}</b><span class={kdaTone(k)}>KDA {one(k)}</span></span>
              <span class="dmg num">{m.plugin_lol_damage({ damage: num(g.damage), dpm: num(g.dpm) })}</span>
              <span class="gear">
                <Items items={g.items} />
                {#if g.augments.length}<span class="augs">{#each g.augments as a (a.id)}<Augment augment={a} size={18} />{/each}</span>{/if}
              </span>
              <span class="when">{g.queue}<br />{when(g.createdMs)}</span>
            </button>
          </li>
        {/each}
      </ol>
    {:else if loading}
      <p class="banner">{m.plugin_lol_loading()}</p>
    {/if}
    {#if display !== "inline"}<Footer data={m.plugin_lol_foot_client()} onLink={session.openLink} />{/if}
  </main>
</div>

<style>
  .search {
    display: flex;
    gap: 8px;
    margin-bottom: var(--lol-sp-5);
  }
  .field {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 6px;
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
  .who {
    display: flex;
    gap: 14px;
    align-items: center;
    margin-bottom: var(--lol-sp-5);
  }
  .avatar {
    position: relative;
    flex: none;
    margin-bottom: 6px;
  }
  .lvl {
    position: absolute;
    left: 50%;
    bottom: -7px;
    z-index: 1;
    transform: translateX(-50%);
    padding: 0 5px;
    border-radius: 999px;
    background: var(--lol-badge-bg);
    color: var(--lol-badge-text);
    font-size: var(--lol-fs-2xs);
    line-height: 13px;
    white-space: nowrap;
  }
  .who-text p {
    margin: 0;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--lol-fs-lg);
    line-height: var(--lol-lh-lg);
    font-weight: var(--lol-fw-strong);
  }
  .tagline {
    margin-left: -6px;
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-3);
  }
  .ranks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
  }
  .ranks span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .sum {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin-top: 2px !important;
    color: var(--lol-text-2);
  }
  .sum b {
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .games {
    margin: 0;
    padding: 0;
    overflow: hidden;
    list-style: none;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-lg);
  }
  .games li + li {
    border-top: 1px solid var(--lol-line);
  }
  .game {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: var(--lol-row-list);
    padding: 4px 12px 4px 4px;
    border: 0;
    background: transparent;
    cursor: pointer;
    text-align: left;
  }
  .game:hover {
    background: var(--lol-hover);
  }
  .res {
    display: grid;
    flex: none;
    place-items: center;
    width: 48px;
    align-self: stretch;
    border-radius: var(--lol-r-md);
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
  }
  .res b {
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-strong);
  }
  .res.win {
    background: var(--lol-win-soft);
  }
  .res.win b {
    color: var(--lol-win);
  }
  .res.loss {
    background: var(--lol-loss-soft);
  }
  .res.loss b {
    color: var(--lol-loss);
  }
  .kda {
    display: grid;
    width: 88px;
    flex: none;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .kda b {
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .dmg {
    width: 150px;
    flex: none;
    white-space: nowrap;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .gear {
    display: grid;
    gap: 2px;
  }
  .augs {
    display: flex;
    gap: 2px;
  }
  .when {
    margin-left: auto;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    text-align: right;
    white-space: nowrap;
  }
  @container (max-width: 719px) {
    .dmg,
    .game :global(.perks) {
      display: none;
    }
  }
  @container (max-width: 519px) {
    .when,
    .game :global(.spells) {
      display: none;
    }
  }
</style>
