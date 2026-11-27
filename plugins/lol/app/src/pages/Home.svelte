<script lang="ts">
  import { onMount } from "svelte";
  import { profileIconPath } from "../assets";
  import ProfileBackground from "../components/ProfileBackground.svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import Pic from "../components/Pic.svelte";
  import RankBadge from "../components/RankBadge.svelte";
  import RecentHistory, { type HistoryPosition } from "./RecentHistory.svelte";
  import StateBlock from "../components/StateBlock.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { one, pctOf, splitRiotId, when } from "../format";
  import { PAGE_SETTINGS, resultStatus, TOOL_PLAYER, TOOL_SCOUT, type Session } from "../host";
  import { m } from "../i18n.svelte";
  import { stateView, type ToolState } from "../status";
  import { isMatchData, isPlayerData, type MatchData, type PlayerData } from "../types";
  import { phaseOf } from "./match/match";

  let { session, history, onTab, onGame }: {
    session: Session;
    history?: HistoryPosition;
    onTab: (page: PageId) => void;
    onGame: (gameId: number, position: HistoryPosition) => void;
  } = $props();

  let player = $state<PlayerData | null>(null);
  let match = $state<MatchData | null>(null);
  let playerState = $state<ToolState | null>(null);
  let matchState = $state<ToolState | null>(null);
  let fetchingPlayer = $state(false);
  let fetchingMatch = $state(false);
  let mounted = false;
  let lastPlayerAttempt = 0;
  let playerRequest = 0;
  let retryIdentity = false;

  const me = $derived(match?.allies.find((member) => member.me));
  const account = $derived(clientUnavailable(playerState) ? null : player ?? me);
  const [name, tag] = $derived(splitRiotId(account?.riotId || me?.name || ""));
  let historyRefresh = $state(0);
  const summary = $derived(player?.summary);
  const mains = $derived(player?.champions?.slice(0, 3) ?? []);
  const phase = $derived(match ? phaseOf(match) ?? "in_game" : matchState?.reason);
  const live = $derived(phase === "champ_select" || phase === "loading" || phase === "in_game");
  const phaseLabel = $derived(
    phase === "in_game" ? m.plugin_lol_phase_in_game()
      : phase === "champ_select" ? m.plugin_lol_phase_select()
      : phase === "loading" ? m.plugin_lol_phase_loading()
      : phase === "post_match" ? m.plugin_lol_phase_post()
      : matchState?.status === "not_in_game" || matchState?.status === "not_in_client" ? stateView(matchState).pill : "",
  );

  function failure(error: unknown): ToolState {
    return { status: "error", text: error instanceof Error ? error.message : String(error) };
  }

  function clientUnavailable(state: ToolState | null) {
    return state?.status === "disabled" || state?.status === "not_in_client" || state?.reason === "client_closed";
  }

  function clearPlayer(state: ToolState | null) {
    ++playerRequest;
    fetchingPlayer = false;
    player = null;
    playerState = state;
    retryIdentity = false;
  }

  async function loadPlayer() {
    if (fetchingPlayer || !mounted) return;
    const request = ++playerRequest;
    fetchingPlayer = true;
    lastPlayerAttempt = Date.now();
    try {
      const result = await session.call(TOOL_PLAYER, { includeProfileBackground: true });
      if (!mounted || request !== playerRequest) return;
      if (isPlayerData(result.structuredContent)) {
        // 客户端可能在战绩请求途中换号。在对局已识别出新账号时不要显示旧账号的返回值。
        if (me?.puuid && me.puuid !== result.structuredContent.puuid) {
          player = null;
          playerState = null;
          retryIdentity = true;
          return;
        }
        player = result.structuredContent;
        playerState = null;
        retryIdentity = false;
      } else {
        playerState = resultStatus(result);
        if (clientUnavailable(playerState)) player = null;
      }
    } catch (error) {
      if (mounted && request === playerRequest) playerState = failure(error);
    } finally {
      if (request === playerRequest) fetchingPlayer = false;
    }
  }

  function connected(state: ToolState | null) {
    return !state || (state.status === "not_in_game" && state.reason !== "client_closed");
  }

  async function loadMatch(forcePlayer = false) {
    if (fetchingMatch || !mounted) return;
    fetchingMatch = true;
    const wasLive = live;
    const wasOffline = !connected(matchState);
    const previousAccount = me?.puuid;
    try {
      const result = await session.call(TOOL_SCOUT);
      if (!mounted) return;
      match = isMatchData(result.structuredContent) ? result.structuredContent : null;
      matchState = match ? null : resultStatus(result);
      if (clientUnavailable(matchState)) {
        clearPlayer(matchState);
        return;
      }
      // 对局阶段按时更新。只有重连、换号、结算或资料暂时缺失时才重读自己的战绩。
      const switched = me?.puuid && ((previousAccount && me.puuid !== previousAccount) || (player && me.puuid !== player.puuid));
      if (switched) {
        clearPlayer(null);
        void loadPlayer();
        return;
      }
      const missing = !player && (retryIdentity || playerState) && playerState?.status !== "disabled" && Date.now() - lastPlayerAttempt >= 30000;
      if (!forcePlayer && connected(matchState) && (wasOffline || (wasLive && !live) || missing)) {
        void loadPlayer();
      }
    } catch (error) {
      if (mounted) {
        match = null;
        matchState = failure(error);
      }
    } finally {
      fetchingMatch = false;
    }
  }

  function refresh() {
    historyRefresh++;
    void loadPlayer();
    void loadMatch(true);
  }

  onMount(() => {
    mounted = true;
    refresh();
    const check = () => { if (document.visibilityState === "visible") void loadMatch(); };
    const resume = () => {
      if (document.visibilityState !== "visible") return;
      check();
      if (Date.now() - lastPlayerAttempt >= 3000) void loadPlayer();
    };
    const timer = setInterval(check, 10000);
    document.addEventListener("visibilitychange", resume);
    window.addEventListener("focus", resume);
    return () => {
      mounted = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("focus", resume);
    };
  });
</script>

<div class="app" data-testid="lol-app-home">
  <TopBar current="home" {onTab} onRefresh={refresh} loading={fetchingMatch || fetchingPlayer} />
  <main class="home" class:has-art={!!player?.profileBackground}>
    {#if player?.profileBackground}
      {#key player.profileBackground}
        <ProfileBackground path={player.profileBackground} />
      {/key}
    {/if}
    <header class="mast">
      <div class="identity">
        {#if account}
          <span class="portrait"><Pic path={account.profileIcon || profileIconPath(account.profileIconId)} name={name} size={56} round /></span>
        {/if}
        <div class="who">
          {#if account && name}
            <span class="eyebrow">{m.plugin_lol_home_profile()}{#if account.summonerLevel}<span class="level num">{m.plugin_lol_level_short({ level: account.summonerLevel })}</span>{/if}</span>
            <h1 data-testid="lol-home-name">{name}{#if tag}<span class="tag">#{tag}</span>{/if}</h1>
            <div class="ranks">
              {#if account.ranked?.solo || account.rank}
                <span>{m.plugin_lol_queue_solo()} <RankBadge ranked={account.ranked?.solo} legacy={account.rank} /></span>
              {/if}
              {#if account.ranked?.flex}
                <span>{m.plugin_lol_queue_flex()} <RankBadge ranked={account.ranked.flex} /></span>
              {/if}
              {#if account.hidden}<span>{m.plugin_lol_hidden()}</span>{/if}
            </div>
          {:else}
            <h1>{m.plugin_lol_home_profile()}</h1>
          {/if}
        </div>
      </div>
      {#if phaseLabel}
        <div class="session" class:active={live}>
          <span class="phase-line"><i aria-hidden="true"></i><strong data-testid="lol-home-phase">{phaseLabel}</strong>{#if live && match?.mode}<span class="mode">{match.mode}</span>{/if}</span>
          {#if live}
            <button type="button" class="enter" data-testid="lol-home-match" onclick={() => onTab("match")}>{m.plugin_lol_home_enter_match()}<span class="next"><Icon name="back" /></span></button>
          {/if}
        </div>
      {/if}
    </header>

    {#if playerState && player}
      <p class="notice" role="status" title={playerState.text}>{stateView(playerState).text}</p>
    {:else if playerState}
      <StateBlock state={playerState} onSettings={() => session.openPage(PAGE_SETTINGS, {})} />
    {:else if !account && fetchingPlayer}
      <p class="notice" role="status">{m.plugin_lol_loading()}</p>
    {/if}
    {#if matchState && !phaseLabel && matchState.status !== playerState?.status && player}
      <p class="notice" role="status" title={matchState.text}>{stateView(matchState).text}</p>
    {:else if matchState && !phaseLabel && matchState.status !== playerState?.status}
      <StateBlock state={matchState} onSettings={() => session.openPage(PAGE_SETTINGS, {})} />
    {/if}

    {#if player}
      {#if (summary && summary.games > 0) || mains.length}
        <div class="overview">
          {#if summary && summary.games > 0}
            <section class="form">
              <h2>{m.plugin_lol_recent_games({ count: summary.games })}</h2>
              <div class="figures num">
                <div class="rate"><strong>{pctOf(summary.wins, summary.games)}<small>%</small></strong><span>{m.plugin_lol_col_win()}</span></div>
                <div class="stat"><b>{m.plugin_lol_wins_losses({ wins: summary.wins, losses: summary.losses })}</b><span>{one(summary.kda)} KDA</span></div>
              </div>
              {#if player.games.length}
                <div class="results" aria-hidden="true">
                  {#each player.games as g (g.gameId)}
                    <span class:won={g.win} title={when(g.createdMs) + " · " + (g.win ? m.plugin_lol_victory() : m.plugin_lol_defeat())}></span>
                  {/each}
                </div>
              {/if}
            </section>
          {/if}
          {#if mains.length}
            <section class="mains">
              <h2>{m.plugin_lol_top_champions()}</h2>
              <ul>
                {#each mains as c (c.champion.id)}
                  <li><ChampIcon id={c.champion.id} icon={c.champion.icon} name={c.champion.short} size={32} /><div><b>{c.champion.short}</b><span class="num">{m.plugin_lol_games_rate({ games: c.games, rate: pctOf(c.wins, c.games) })}</span></div></li>
                {/each}
              </ul>
            </section>
          {/if}
        </div>
      {/if}
      {#key player.puuid}
        <RecentHistory {session} {player} restore={history} refreshVersion={historyRefresh} {onGame} onHistory={() => onTab("player")} />
      {/key}
    {/if}
    <Footer sources={[{ id: "client" }]} onLink={session.openLink} />
  </main>
</div>

<style>
  .home {
    position: relative;
    isolation: isolate;
    max-width: 1160px;
    margin: 0 auto;
    padding: clamp(14px, 3cqi, 34px) clamp(16px, 3.6cqi, 44px) 16px;
  }
  .has-art .mast { min-height: 150px; }
  h1, h2, p, ul { margin: 0; }
  h1, h2, b, strong { font-weight: var(--lol-fw-strong); }
  h1 { font-size: clamp(24px, 3.8cqi, 42px); letter-spacing: -.045em; line-height: 1.22; overflow-wrap: anywhere; }
  h2 { font-size: var(--lol-fs-md); line-height: 1.45; }
  .mast, .identity, .ranks, .ranks > span, .phase-line, .figures, .enter, .next, li {
    display: flex;
    align-items: center;
  }
  .mast { justify-content: space-between; flex-wrap: wrap; gap: 22px; padding: 2px 0 24px; border-bottom: 1px solid var(--lol-line-2); }
  .identity { flex: 1 1 220px; min-width: 0; gap: 20px; }
  .portrait { flex: none; display: grid; padding: 4px; border: 1px solid var(--lol-line-2); border-radius: 50%; }
  .portrait :global(.ci) { --s: clamp(56px, 7.5cqi, 84px); }
  .portrait :global(.ph) { font-size: 1.8em; }
  .who { min-width: 0; }
  .eyebrow, .ranks, .phase-line, small, .mains li span.num, .notice { color: var(--lol-text-2); }
  .eyebrow { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 3px; font-size: var(--lol-fs-xs); }
  .level { padding-left: 10px; border-left: 1px solid var(--lol-line-2); line-height: 1; }
  .tag { margin-left: .25em; font-size: .50em; color: var(--lol-text-3); font-weight: var(--lol-fw-normal); letter-spacing: 0; }
  .ranks { flex-wrap: wrap; gap: 4px 16px; font-size: var(--lol-fs-xs); }
  .ranks:not(:empty) { margin-top: 6px; }
  .ranks > span { gap: 5px; }
  .session { display: grid; justify-items: end; gap: 10px; }
  .phase-line { flex-wrap: wrap; gap: 7px; font-size: var(--lol-fs-xs); }
  .phase-line i { width: 6px; height: 6px; border-radius: 50%; background: var(--lol-text-3); }
  .active .phase-line i { background: var(--lol-ally); }
  .phase-line strong { font-weight: var(--lol-fw-medium); }
  .mode { padding-left: 8px; border-left: 1px solid var(--lol-line); }
  .home button { cursor: pointer; }
  .enter { justify-content: space-between; gap: 24px; padding: 8px 13px; border: 0; border-radius: var(--lol-r-sm); background: var(--lol-ink-fill); color: var(--lol-ink-on); font-weight: var(--lol-fw-strong); }
  .enter:hover { opacity: .86; }
  .next { flex: none; transform: rotate(180deg); }
  .overview { display: flex; padding: 24px 0; border-bottom: 1px solid var(--lol-line); gap: 28px; }
  .overview section { flex: 1; min-width: 0; }
  .overview section + section { border-left: 1px solid var(--lol-line); padding-left: 28px; flex: 1.2; }
  .figures { flex-wrap: wrap; gap: 8px 20px; margin-top: 13px; }
  .rate { display: flex; align-items: baseline; gap: 8px; }
  .rate strong { font-size: clamp(30px, 4cqi, 48px); letter-spacing: -.045em; line-height: 1; }
  .rate small { font-size: .50em; margin-left: 2px; color: inherit; }
  .rate > span, .stat span, .mains li span.num, small { font-size: var(--lol-fs-xs); }
  .rate > span, .stat span { color: var(--lol-text-2); }
  .stat, li > div { display: grid; gap: 4px; min-width: 0; }
  .results { display: flex; gap: 3px; max-width: 315px; margin-top: 13px; }
  .results span { height: 5px; flex: 1; background: var(--lol-loss); opacity: .75; }
  .results .won { background: var(--lol-win); }
  ul { padding: 0; list-style: none; }
  .mains ul { display: flex; flex-wrap: wrap; gap: 12px 18px; margin-top: 19px; }
  li { gap: 7px; }
  li b { overflow-wrap: anywhere; }
  .notice { padding: 18px 0; }
  @container (max-width: 640px) {
    .identity { gap: 12px; }
    .overview { flex-direction: column; gap: 18px; padding: 18px 0; }
    .overview section + section { border: 0; padding-left: 0; }
    .mains ul { margin-top: 10px; }
  }
  @container (max-width: 460px) {
    .mast { align-items: flex-start; gap: 14px; padding-bottom: 16px; }
    .identity { flex-basis: 100%; }
    .session { width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    .enter { flex-shrink: 0; gap: 8px; padding: 6px 9px; }
    .mains ul { gap: 9px 12px; }
  }
</style>
