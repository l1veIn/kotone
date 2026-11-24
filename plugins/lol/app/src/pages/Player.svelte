<script lang="ts">
  // 战绩（`ui://kotone/lol/player`，`lol.player` 的页面；docs/design/lol-app.md 第三节 2，调研 G33、G34）。
  // 全屏：搜人；头部（召唤师头像 56 圆形 + 等级、名字#编号、隐藏战绩、单双排 / 灵活组排的段位和胜负、最高段位）；
  // 近 20 局的胜负、KDA、场均死、连胜连败；常玩的英雄（最多 5 个）；每局一行 44 px（有海克斯 52 px）：
  // 结果格 · 英雄（右下英雄等级）· 两个召唤师技能 · 符文 · KDA · 伤害 · 装备 6 + 1（下面一排海克斯）· 模式和时间。
  // 窄时每局两行。点一局看单局详情。卡片（inline，她调 `lol.player` 时）：头部 + 最近 5 局。
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
  import StateBlock from "../components/StateBlock.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { profileIconPath } from "../assets";
  import { duration, kdaOf, kdaTone, num, one, pctOf, rateTone, splitRiotId, tierName, when } from "../format";
  import { PAGE_SETTINGS, resultStatus, TOOL_PLAYER, type DisplayMode, type Session, type ToolResult } from "../host";
  import { m } from "../i18n.svelte";
  import { stateView, type ToolState } from "../status";
  import { isPlayerData, type PlayerData, type Ranked } from "../types";

  let {
    session,
    initial,
    display,
    args = {},
    onTab,
    onGame,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    /** `{ puuid }`（这局里点一个人）或 `{ player }`（名字#编号）；空 = 自己。 */
    args?: Record<string, unknown>;
    onTab: (page: PageId) => void;
    onGame: (gameId: number) => void;
  } = $props();

  let player = $state<PlayerData | null>(null);
  let status = $state<ToolState | null>(null);
  let error = $state("");
  let loading = $state(false);
  let query = $state("");
  let lastArgs: Record<string, unknown> = {};

  const full = $derived(display !== "inline");
  const canOpen = $derived(!!session.context?.availableDisplayModes?.includes("fullscreen"));

  function show(result: ToolResult) {
    if (isPlayerData(result.structuredContent)) {
      player = result.structuredContent;
      status = null;
    } else {
      status = resultStatus(result);
    }
  }

  /** `first`：宿主没推结果时取的第一份（推得晚的宿主推来了就用推来的，见 host.ts 的 firstCall）。 */
  async function load(next: Record<string, unknown>, first = false) {
    lastArgs = next;
    loading = true;
    error = "";
    try {
      show(await (first ? session.firstCall : session.call)(TOOL_PLAYER, next));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    const own = typeof args.puuid === "string" && args.puuid ? { puuid: args.puuid } : typeof args.player === "string" && args.player ? { player: args.player } : null;
    if (initial) void initial.then((result) => (result ? show(result) : load(own ?? session.input() ?? {}, true)));
    else void load(own ?? {});
  });

  function search(event: SubmitEvent) {
    event.preventDefault();
    const q = query.trim();
    void load(q ? { player: q } : {});
  }

  const s = $derived(player?.summary && player.summary.games > 0 ? player.summary : null);
  const [nm, tag] = $derived(splitRiotId(player?.riotId ?? ""));
  const streak = $derived(
    s && s.streak >= 3 ? m.plugin_lol_tag_win_streak({ count: s.streak }) : s && s.streak <= -3 ? m.plugin_lol_tag_loss_streak({ count: -s.streak }) : "",
  );
  /** 段位那一段：“钻石 II · 45 点 · 52 胜 48 负 · 最高 大师”。 */
  const rankDetail = (r: Ranked) =>
    [
      r.tier && (r.wins || r.losses) ? m.plugin_lol_wins_losses({ wins: r.wins, losses: r.losses }) : "",
      r.highestTier ? m.plugin_lol_rank_highest({ tier: tierName(r.highestTier) }) : "",
    ]
      .filter(Boolean)
      .join(" · ");
  const CARD_GAMES = 5;
</script>

{#snippet avatar(size: number)}
  {#if player}
    <span class="avatar" style:--size="{size}px">
      <Pic path={player.profileIcon || profileIconPath(player.profileIconId)} name={nm || "?"} {size} round />
      {#if player.summonerLevel && size >= 40}<span class="lvl num">{m.plugin_lol_level_short({ level: player.summonerLevel })}</span>{/if}
    </span>
  {/if}
{/snippet}

{#if full}
  <div class="app" data-testid="lol-app-player">
    <TopBar current="player" {onTab} onRefresh={() => void load(lastArgs)} {loading} />
    <main class="page">
      <form class="search" onsubmit={search}>
        <span class="field"><Icon name="search" /><input bind:value={query} placeholder={m.plugin_lol_search_placeholder()} aria-label={m.plugin_lol_search_placeholder()} data-testid="lol-player-search" /></span>
        <button type="submit" class="btn primary" disabled={loading} data-testid="lol-player-search-go">{m.plugin_lol_search()}</button>
        <button type="button" class="btn" disabled={loading} data-testid="lol-player-me" onclick={() => { query = ""; void load({}); }}>{m.plugin_lol_search_me()}</button>
      </form>
      {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
      {#if status}
        {#if player}
          <p class="banner" data-testid="lol-app-status-banner" title={status.text}><Icon name="info" />{stateView(status).text}</p>
        {:else}
          <StateBlock state={status} onSettings={() => session.openPage(PAGE_SETTINGS, {})} />
        {/if}
      {/if}
      {#if player}
        <header class="head" data-testid="lol-player-head">
          {@render avatar(56)}
          <div class="head-text">
            <p class="name">
              <span data-testid="lol-player-name">{nm}{#if tag}<span class="tagline">#{tag}</span>{/if}</span>
              {#if player.hidden}<span class="chip warn" data-testid="lol-player-hidden"><Icon name="eyeOff" />{m.plugin_lol_hidden()}</span>{/if}
            </p>
            <p class="ranks">
              {#if player.ranked?.solo}
                <span data-testid="lol-player-rank-solo"><span class="q">{m.plugin_lol_queue_solo()}</span><RankBadge ranked={player.ranked.solo} />
                  {#if rankDetail(player.ranked.solo)}<span class="more">· {rankDetail(player.ranked.solo)}</span>{/if}</span>
              {/if}
              {#if player.ranked?.flex}
                <span><span class="q">{m.plugin_lol_queue_flex()}</span><RankBadge ranked={player.ranked.flex} /></span>
              {/if}
              {#if !player.ranked?.solo && !player.ranked?.flex && player.rank}<span>{m.plugin_lol_queue_solo()} {player.rank}</span>{/if}
            </p>
          </div>
        </header>
        {#if s}
          <p class="sum num" data-testid="lol-player-summary">
            <span class="k">{m.plugin_lol_recent_games({ count: s.games })}</span>
            <b>{m.plugin_lol_wins_losses({ wins: s.wins, losses: s.losses })}</b>
            <b class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</b>
            <span>KDA <b class={kdaTone(s.kda)}>{one(s.kda)}</b></span>
            <span>{m.plugin_lol_avg_deaths()} <b>{one(s.deaths)}</b></span>
            {#if streak}<span class="chip {s.streak > 0 ? 'good' : 'bad'}">{streak}</span>{/if}
          </p>
        {/if}
        {#if player.champions.length}
          <p class="mains" data-testid="lol-player-champions">
            <span class="k">{m.plugin_lol_top_champions()}</span>
            {#each player.champions.slice(0, 5) as c (c.champion.id)}
              <span class="main"><ChampIcon id={c.champion.id} icon={c.champion.icon} name={c.champion.short} size={24} />
                <span>{c.champion.short}</span><span class="num muted">{m.plugin_lol_games_rate({ games: c.games, rate: pctOf(c.wins, c.games) })}</span></span>
            {/each}
          </p>
        {/if}
        {#if player.error && !player.games.length}<p class="banner" title={player.error}><Icon name="info" />{m.plugin_lol_lookup_failed({ error: player.error })}</p>{/if}
        {#if player.games.length}
          <ol class="games" data-testid="lol-player-games">
            {#each player.games as g (g.gameId)}
              {@const k = kdaOf(g.kills, g.deaths, g.assists)}
              <li>
                <button type="button" class="game" class:augs={g.augments.length > 0} onclick={() => onGame(g.gameId)} data-testid="lol-player-game" data-game-id={g.gameId}
                  title={m.plugin_lol_open_game_title()}>
                  <span class="res {g.win ? 'win' : 'loss'}"><b>{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</b><span class="num">{duration(g.durationS)}</span></span>
                  <ChampIcon id={g.champion.id} icon={g.champion.icon} name={g.champion.short} level={g.level} />
                  <span class="wide-only"><Spells spells={g.spells} /></span>
                  <span class="wide-only"><Perks perks={g.perks} /></span>
                  <span class="kda num">
                    <b>{g.kills} / <span class="bad">{g.deaths}</span> / {g.assists}</b>
                    <span>KDA <span class={kdaTone(k)}>{one(k)}</span></span>
                  </span>
                  <span class="dmg num">
                    <span>{m.plugin_lol_damage_short({ damage: num(g.damage) })}</span>
                    <span class="muted">{m.plugin_lol_dpm_short({ dpm: num(g.dpm) })}</span>
                  </span>
                  <span class="gear">
                    <Items items={g.items} size={22} />
                    {#if g.augments.length}<span class="aug-row">{#each g.augments as a (a.id)}<Augment augment={a} size={18} />{/each}</span>{/if}
                  </span>
                  <span class="when"><span>{g.queue}</span><span class="num">{when(g.createdMs)}</span></span>
                </button>
              </li>
            {/each}
          </ol>
        {:else if !player.error}
          <p class="banner"><Icon name="info" />{m.plugin_lol_no_games()}</p>
        {/if}
      {:else if loading}
        <div class="skel-head" aria-busy="true" aria-label={m.plugin_lol_loading()}><span class="ci round s56"></span><span class="skel" style:width="160px"></span></div>
      {/if}
      <Footer data={m.plugin_lol_foot_client()} onLink={session.openLink} />
    </main>
  </div>
{:else}
  <div class="app card" data-testid="lol-app-player">
    {#if player}
      <div class="card-head">
        {@render avatar(32)}
        <span class="t">{nm}{#if tag}<span class="tagline">#{tag}</span>{/if}</span>
        <RankBadge ranked={player.ranked?.solo} legacy={player.rank} />
        {#if player.hidden}<span class="chip warn"><Icon name="eyeOff" />{m.plugin_lol_hidden()}</span>{/if}
        {#if canOpen}
          <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} data-testid="lol-card-open"
            onclick={() => void session.requestDisplayMode("fullscreen")}><Icon name="open" /></button>
        {/if}
      </div>
      {#if s}
        <p class="card-sum num">
          {m.plugin_lol_recent_games({ count: s.games })} {m.plugin_lol_wins_losses({ wins: s.wins, losses: s.losses })}
          <span class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</span> · KDA {one(s.kda)}{#if streak} · {streak}{/if}
        </p>
      {/if}
      <div class="card-games">
        {#each player.games.slice(0, CARD_GAMES) as g (g.gameId)}
          <div class="cg num" data-testid="lol-card-game">
            <b class={g.win ? "good" : "bad"}>{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</b>
            <ChampIcon id={g.champion.id} icon={g.champion.icon} name={g.champion.short} size={20} />
            <span class="cn">{g.champion.short}</span>
            <span>{g.kills}/{g.deaths}/{g.assists}</span>
            <span class="muted cw">{g.queue} · {when(g.createdMs)}</span>
          </div>
        {/each}
      </div>
      <Footer data="" card extra={player.games.length > CARD_GAMES ? m.plugin_lol_card_more({ count: player.games.length - CARD_GAMES }) : ""} onLink={session.openLink} />
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
  .avatar {
    position: relative;
    flex: none;
    width: var(--size);
    height: var(--size);
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
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: var(--lol-sp-5);
  }
  .head-text {
    min-width: 0;
  }
  .head-text p {
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
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-3);
  }
  .ranks {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 20px;
    margin-top: 2px !important;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .ranks > span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .ranks .q,
  .k {
    color: var(--lol-text-3);
  }
  .ranks .more {
    color: var(--lol-text-3);
  }
  .sum,
  .mains {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
    margin: 0 0 var(--lol-sp-4);
    color: var(--lol-text-2);
  }
  .sum b {
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .main {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .muted {
    color: var(--lol-text-3);
  }
  .games {
    margin: var(--lol-sp-5) 0 0;
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
  .game.augs {
    min-height: 52px;
  }
  .game:hover {
    background: var(--lol-hover);
  }
  .wide-only {
    display: contents;
  }
  .res {
    display: grid;
    flex: none;
    place-content: center;
    justify-items: center;
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
    flex: none;
    width: 92px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .kda b {
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .dmg {
    display: grid;
    flex: none;
    width: 112px;
    font-size: var(--lol-fs-xs);
    white-space: nowrap;
  }
  .gear {
    display: grid;
    gap: 2px;
  }
  .aug-row {
    display: flex;
    gap: 2px;
  }
  .when {
    display: grid;
    margin-left: auto;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    text-align: right;
    white-space: nowrap;
  }
  .skel-head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  @container (max-width: 759px) {
    .dmg {
      display: none;
    }
  }
  @container (max-width: 519px) {
    .game {
      flex-wrap: wrap;
      gap: 4px 8px;
      min-height: 52px;
    }
    .res {
      width: 28px;
    }
    .res span,
    .wide-only,
    .when {
      display: none;
    }
    .game :global(.items .ci) {
      --s: 18px !important;
    }
    .search .btn:last-child {
      display: none;
    }
  }
  /* 卡片 */
  .card .card-head .t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-sum {
    margin: 0 12px 4px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .card-games {
    padding: 0 12px;
  }
  .cg {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    white-space: nowrap;
  }
  .cg + .cg {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .cn {
    width: 4.6em;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--lol-fw-medium);
  }
  .cw {
    min-width: 0;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--lol-fs-xs);
  }
</style>
