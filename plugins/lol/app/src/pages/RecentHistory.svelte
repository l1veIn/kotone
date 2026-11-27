<script lang="ts" module>
  export interface HistoryPosition {
    puuid: string;
    pageSize: 10 | 20;
    offsets: number[];
    gameId: number;
  }
</script>

<script lang="ts">
  import { tick, untrack } from "svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Icon from "../components/Icon.svelte";
  import Pager from "../components/Pager.svelte";
  import { duration, when } from "../format";
  import { resultStatus, TOOL_HISTORY, type Session } from "../host";
  import { m } from "../i18n.svelte";
  import { stateView, type ToolState } from "../status";
  import { isHistoryPage, type HistoryPage, type PlayerData } from "../types";

  let { session, player, restore, refreshVersion, onGame, onHistory }: {
    session: Session;
    player: PlayerData;
    restore?: HistoryPosition;
    refreshVersion: number;
    onGame: (gameId: number, history: HistoryPosition) => void;
    onHistory: () => void;
  } = $props();

  // 下一页从第几局开始由 lol.history 给（客户端给的那一段里切页，重开、自定义不算）。
  const saved = untrack(() => restore?.puuid === player.puuid ? restore : undefined);
  let offsets = $state<number[]>(saved?.offsets ?? [0]);
  let pageSize = $state<10 | 20>(saved?.pageSize ?? 10);
  let offset = $derived(offsets[offsets.length - 1]);
  let accountId = $derived(player.puuid);
  let firstGameId = $derived(player.games[0]?.gameId);
  let section: HTMLElement;
  let page = $state<HistoryPage | null>(null);
  let failure = $state<ToolState | null>(null);
  let loading = $state(true);
  let retry = $state(0);
  let focusGameId = saved?.gameId;
  let scrollToList = false;

  $effect(() => {
    const puuid = accountId;
    const request = { puuid, offset, pageSize };
    // 手动刷新或有新对局时，重新读取当前页。
    void refreshVersion; void firstGameId; void retry;
    let current = true;
    loading = true;
    failure = null;
    async function load() {
      try {
        const result = await session.call(TOOL_HISTORY, request);
        if (!current) return;
        const data = result.structuredContent;
        if (!isHistoryPage(data) || data.puuid !== puuid || data.offset !== request.offset || data.pageSize !== request.pageSize) {
          failure = resultStatus(result);
          return;
        }
        page = data;
      } catch (error) {
        if (current) failure = { status: "error", text: error instanceof Error ? error.message : String(error) };
      } finally {
        if (current) {
          loading = false;
          await tick();
          if (current && !failure) {
            if (focusGameId !== undefined) {
              const row = section.querySelector<HTMLButtonElement>(`[data-game-id="${focusGameId}"]`);
              row?.focus({ preventScroll: true });
              row?.scrollIntoView({ block: "nearest" });
              focusGameId = undefined;
            } else if (scrollToList) {
              section.scrollIntoView({ block: "start" });
              scrollToList = false;
            }
          }
        }
      }
    }
    void load();
    return () => { current = false; };
  });

  function resize(event: Event) {
    const count = (event.currentTarget as HTMLSelectElement).value;
    pageSize = count === "20" ? 20 : 10;
    offsets = [0];
    scrollToList = true;
  }

  function next() {
    if (page?.nextOffset == null || loading) return;
    offsets = [...offsets, page.nextOffset];
    scrollToList = true;
  }

  function previous() {
    if (offsets.length === 1 || loading) return;
    offsets = offsets.slice(0, -1);
    scrollToList = true;
  }

  function open(gameId: number) {
    onGame(gameId, { puuid: player.puuid, offsets: [...offsets], pageSize, gameId });
  }
</script>

<section class="history" bind:this={section} data-testid="lol-home-recent" aria-label={m.plugin_lol_home_recent()}>
  <header class="section-head">
    <h2>{m.plugin_lol_home_recent()}</h2>
    <select value={pageSize} onchange={resize} disabled={loading} aria-label={m.plugin_lol_history_page_size()} data-testid="lol-home-page-size">
      <option value={10}>{m.plugin_lol_history_per_page({ count: 10 })}</option>
      <option value={20}>{m.plugin_lol_history_per_page({ count: 20 })}</option>
    </select>
    <button class="history-link" type="button" data-testid="lol-home-history" onclick={onHistory}>
      {m.plugin_lol_home_history()}<span class="forward"><Icon name="back" /></span>
    </button>
  </header>
  <div aria-busy={loading}>
    {#if failure}
      <p class="notice" role="status" title={failure.text}>{stateView(failure).text}
        <button class="btn" type="button" onclick={() => retry++} data-testid="lol-home-history-retry">{m.plugin_lol_retry()}</button>
      </p>
    {:else if page?.games.length}
      <div class="columns grid" aria-hidden="true">
        <span>{m.plugin_lol_history_result()}</span><span>{m.plugin_lol_col_champion()}</span>
        <span>K / D / A</span><span>{m.plugin_lol_history_duration()}</span><span>{m.plugin_lol_history_started()}</span>
      </div>
      <ul class="games" class:pending={loading}>
        {#each page.games as game (game.gameId)}
          <li>
            <button type="button" class="row grid" data-testid="lol-home-game" data-game-id={game.gameId}
              disabled={loading} onclick={() => open(game.gameId)} title={m.plugin_lol_open_game_title()}>
              <span class="result" class:won={game.win}>{game.win ? m.plugin_lol_victory() : m.plugin_lol_defeat()}</span>
              <span class="pick"><ChampIcon id={game.champion.id} icon={game.champion.icon} name={game.champion.short} size={32} />
                <span><b>{game.champion.short}</b><small>{game.queue}</small></span>
              </span>
              <span class="score num">{game.kills} <i>/</i> {game.deaths} <i>/</i> {game.assists}</span>
              <span class="length num">{duration(game.durationS)}</span>
              <span class="started num">{when(game.createdMs)}</span>
              <span class="forward row-arrow"><Icon name="back" /></span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="notice" role="status">{loading ? m.plugin_lol_loading() : player.hidden ? m.plugin_lol_hidden() : m.plugin_lol_no_games()}</p>
    {/if}
  </div>
  <Pager id="home" label={m.plugin_lol_home_recent()} page={offsets.length} canPrev={!loading && offsets.length > 1}
    canNext={!loading && !failure && !!page?.hasMore} clientGames={!loading && !failure && page && !page.hasMore && page.games.length ? page.clientGames : 0}
    onPrev={previous} onNext={next} />
</section>

<style>
  .history { margin-top: 22px; scroll-margin-top: 48px; }
  .section-head, .history-link, .forward, .pick { display: flex; align-items: center; }
  .section-head { gap: 12px; padding-bottom: 10px; }
  h2 { flex: 1; margin: 0; font-size: var(--lol-fs-md); font-weight: var(--lol-fw-strong); }
  .history-link { gap: 3px; padding: 4px 0; color: var(--lol-text-2); border: 0; background: transparent; cursor: pointer; font-size: var(--lol-fs-xs); }
  .history-link:hover { color: var(--lol-text); text-decoration: underline; text-underline-offset: 3px; }
  select { color: var(--lol-text); background: var(--lol-bg-2); border: 1px solid var(--lol-line-2); border-radius: var(--lol-r-sm); padding: 4px 5px; font: inherit; font-size: var(--lol-fs-xs); cursor: pointer; }
  .forward { transform: rotate(180deg); }
  .grid { display: grid; grid-template-columns: minmax(48px, .5fr) minmax(135px, 2.2fr) minmax(85px, 1.1fr) minmax(54px, .7fr) minmax(86px, 1fr) 14px; align-items: center; gap: 10px; padding-inline: 10px; }
  .columns { min-height: 27px; color: var(--lol-text-2); background: var(--lol-bg-2); border-block: 1px solid var(--lol-line); font-size: var(--lol-fs-xs); }
  .games { margin: 0; padding: 0; list-style: none; }
  .pending { opacity: .6; }
  .row { width: 100%; padding-block: 6px; min-height: 45px; color: var(--lol-text); border: 0; border-bottom: 1px solid var(--lol-line); background: transparent; font: inherit; text-align: left; cursor: pointer; }
  .row:hover:not(:disabled) { background: var(--lol-bg-2); }
  .result { color: var(--lol-loss); padding-inline-start: 8px; border-inline-start: 2px solid currentColor; font-weight: var(--lol-fw-strong); }
  .result.won { color: var(--lol-win); }
  .pick { min-width: 0; gap: 9px; }
  .pick > span { min-width: 0; display: grid; gap: 1px; }
  .pick b, .pick small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick b, .score { font-weight: var(--lol-fw-strong); }
  .pick small, .started, .length, .row-arrow, .score i, .notice { color: var(--lol-text-2); }
  .pick small, .started, .length { font-size: var(--lol-fs-xs); }
  .score { white-space: nowrap; }
  .score i { font-style: normal; font-weight: var(--lol-fw-normal); }
  .notice { display: flex; align-items: center; gap: 12px; margin: 0; padding-block: 16px; }
  @container (max-width: 590px) {
    .columns { display: none; }
    .games { border-top: 1px solid var(--lol-line); }
    .row { grid-template-columns: auto minmax(0, 1fr) auto; gap: 3px 8px; padding: 6px 2px; }
    .result { grid-row: 1 / 3; padding-inline-start: 5px; }
    .pick { grid-row: 1 / 3; gap: 6px; }
    .score { grid-column: 3; justify-self: end; }
    .length { grid-row: 2; grid-column: 3; justify-self: start; }
    .started { grid-row: 2; grid-column: 3; justify-self: end; padding-inline-start: 35px; }
    .row-arrow { display: none; }
    .section-head { gap: 8px; flex-wrap: wrap; }
  }
</style>
