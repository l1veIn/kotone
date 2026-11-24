<script lang="ts">
  // 单局详情（`ui://kotone/lol/game`，`lol.game` 的页面，只给页面用；docs/design/lol-app.md 第三节 3，调研 G29、G34）。
  // 全屏：标题行（刚结束的小牌、胜利 / 失败、模式、时长、开局时间）；两队各一块（我方 / 对面，看别人的对局时是胜方 / 负方、
  // 灰边条），表头有击杀、伤害合计；每人一行 44 px（有海克斯 52 px）：英雄（右下英雄等级）· 技能 · 符文 · 名字（点开看战绩）·
  // KDA · 伤害条（按这一局最高伤害）· 承伤 · 经济 · 补刀 · 装备 6 + 1（下面一排海克斯）。我那一行淡蓝底 + “我”。
  // 卡片（inline）“这局总结”（自动卡片、`justEnded`）：我这一局的数 + 两队每人一格（头像、K/D/A、伤害短条）。
  // 只写算出来的事实（“全队第 1”），不评价人。
  import { onMount } from "svelte";
  import Augment from "../components/Augment.svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import Items from "../components/Items.svelte";
  import Perks from "../components/Perks.svelte";
  import Spells from "../components/Spells.svelte";
  import StateBlock from "../components/StateBlock.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { duration, kdaOf, kdaTone, num, one, splitRiotId, when } from "../format";
  import { resultStatus, TOOL_GAME, type DisplayMode, type Session, type ToolResult } from "../host";
  import { m } from "../i18n.svelte";
  import type { ToolState } from "../status";
  import { isGameData, type GameData, type GamePlayer, type GameTeam } from "../types";

  let {
    session,
    initial,
    display,
    args = {},
    onTab,
    onPlayer,
    onBack,
    backTo,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    /** `{ gameId }`。 */
    args?: Record<string, unknown>;
    onTab: (page: PageId) => void;
    onPlayer: (puuid: string) => void;
    /** 从战绩点进来时左上角“← 战绩”。 */
    onBack?: () => void;
    /** 回到哪一页（返回按钮上的字）。 */
    backTo?: PageId;
  } = $props();

  let game = $state<GameData | null>(null);
  let status = $state<ToolState | null>(null);
  let error = $state("");
  let loading = $state(false);
  let gameId = 0;

  const full = $derived(display !== "inline");
  const canOpen = $derived(!!session.context?.availableDisplayModes?.includes("fullscreen"));

  function show(result: ToolResult) {
    if (isGameData(result.structuredContent)) {
      game = result.structuredContent;
      gameId = game.gameId;
      status = null;
    } else {
      status = resultStatus(result);
    }
  }

  async function load() {
    const id = gameId || Number(args.gameId ?? session.input()?.gameId ?? 0);
    if (!id) return;
    gameId = id;
    loading = true;
    error = "";
    try {
      show(await session.call(TOOL_GAME, { gameId: id }));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then((result) => (result ? show(result) : load()));
    else void load();
  });

  const players = $derived(game?.teams.flatMap((t) => t.players) ?? []);
  const top = $derived(Math.max(1, ...players.map((p) => p.damage)));
  const me = $derived(players.find((p) => p.me) ?? null);
  /** 我在的那一队（看别人的对局时没有）。 */
  const mine = $derived(game?.teams.find((t) => t.ally || t.players.some((p) => p.me)) ?? null);
  const teams = $derived(
    game ? (mine ? [mine, ...game.teams.filter((t) => t !== mine)] : [...game.teams].sort((a, b) => Number(b.win) - Number(a.win))) : [],
  );
  /** 刚结束：数据里写着（自动卡片），或者从“这局”结算的“看这局详情”点进来。 */
  const justEnded = $derived(!!game?.justEnded || args.justEnded === true);
  const minutes = $derived(Math.max(1, (game?.durationS ?? 60) / 60));

  const side = (t: GameTeam) => (!mine ? "neutral" : t === mine ? "ally" : "enemy");
  const teamName = (t: GameTeam) =>
    !mine ? (t.win ? m.plugin_lol_team_win() : m.plugin_lol_team_loss()) : t === mine ? m.plugin_lol_allies() : m.plugin_lol_enemies();
  const sum = (t: GameTeam, f: (p: GamePlayer) => number) => t.players.reduce((a, p) => a + f(p), 0);
  /** 我的伤害在我们队排第几。 */
  const myDamageRank = $derived(
    me && mine ? [...mine.players].sort((a, b) => b.damage - a.damage).findIndex((p) => p.puuid === me.puuid) + 1 : 0,
  );
</script>

{#snippet outcome(win: boolean)}
  <b class="outcome {win ? 'good' : 'bad'}">{win ? m.plugin_lol_victory() : m.plugin_lol_defeat()}</b>
{/snippet}

{#if full}
  <div class="app" data-testid="lol-app-game" data-just-ended={justEnded ? "true" : "false"}>
    <TopBar current={backTo === "match" ? "match" : "player"} {onTab} onRefresh={() => void load()} {loading} />
    <main class="page">
      {#if onBack}
        <button type="button" class="back" data-testid="lol-game-back" onclick={onBack}><Icon name="back" />{backTo === "match" ? m.plugin_lol_page_match() : m.plugin_lol_page_player()}</button>
      {/if}
      {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
      {#if status && !game}<StateBlock state={status} />{/if}
      {#if game}
        <header class="title" data-testid="lol-game-title">
          {#if justEnded}<span class="tag-ink" data-testid="lol-game-just-ended">{m.plugin_lol_just_ended()}</span>{/if}
          {#if mine}{@render outcome(mine.win)}<span class="sep">·</span>{/if}
          <span class="num">{game.queue} · {duration(game.durationS)} · {when(game.createdMs)}</span>
        </header>
        {#each teams as t (t.teamId)}
          <section class="team {side(t)}" data-testid="lol-game-team" data-side={side(t)} data-win={t.win ? "true" : "false"}>
            <div class="team-head">
              <span class="label"><span class="mark" class:diamond={side(t) === "enemy"}></span>{teamName(t)}
                {#if mine}<span class="wl {t.win ? 'good' : 'bad'}">· {t.win ? m.plugin_lol_victory() : m.plugin_lol_defeat()}</span>{/if}</span>
              <span class="stats num">
                <span>{m.plugin_lol_team_kills({ kills: sum(t, (p) => p.kills) })}</span>
                <span>{m.plugin_lol_team_damage({ damage: num(sum(t, (p) => p.damage)) })}</span>
              </span>
            </div>
            <div class="grid cols">
              <span></span><span>{m.plugin_lol_col_player()}</span><span>KDA</span><span>{m.plugin_lol_col_damage()}</span>
              <span class="r xcol">{m.plugin_lol_col_taken()}</span><span class="r xcol">{m.plugin_lol_col_gold()}</span><span class="r xcol">{m.plugin_lol_col_cs()}</span>
              <span>{m.plugin_lol_col_items()}</span>
            </div>
            {#each t.players as p, i (p.puuid || i)}
              {@const k = kdaOf(p.kills, p.deaths, p.assists)}
              {@const [nm, tag] = splitRiotId(p.riotId)}
              <div class="grid row stripe" class:me={p.me} class:augs={p.augments.length > 0} data-testid="lol-game-player"
                title={m.plugin_lol_game_more({ taken: num(p.taken), gold: num(p.gold), cs: p.cs })}>
                <span class="pics">
                  <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={32} level={p.level} />
                  <span class="wide"><Spells spells={p.spells} size={15} /></span>
                  <span class="wide"><Perks perks={p.perks} size={15} sub={12} /></span>
                </span>
                <span class="who">
                  <button type="button" class="name" data-testid="lol-game-player-open" disabled={!p.puuid} onclick={() => onPlayer(p.puuid)} title={m.plugin_lol_open_player({ name: p.riotId })}>
                    {nm}{#if tag}<span class="tagline">#{tag}</span>{/if}
                  </button>
                  {#if p.me}<span class="chip me">{m.plugin_lol_me_tag()}</span>{/if}
                </span>
                <span class="kda num"><b>{p.kills}/<span class="bad">{p.deaths}</span>/{p.assists}</b> <span class={kdaTone(k)}>{one(k)}</span></span>
                <span class="dmg num"><span class="bar"><span style:width="{Math.round((p.damage / top) * 100)}%"></span></span>{num(p.damage)}</span>
                <span class="r num muted xcol">{num(p.taken)}</span>
                <span class="r num muted xcol">{num(p.gold)}</span>
                <span class="r num muted xcol">{p.cs}</span>
                <span class="gear">
                  <Items items={p.items} size={20} />
                  {#if p.augments.length}<span class="aug-row">{#each p.augments as a (a.id)}<Augment augment={a} size={18} />{/each}</span>{/if}
                </span>
              </div>
            {/each}
          </section>
        {/each}
      {:else if loading}
        <p class="banner" aria-busy="true">{m.plugin_lol_loading()}</p>
      {/if}
      <Footer data={m.plugin_lol_foot_client()} onLink={session.openLink} />
    </main>
  </div>
{:else}
  <div class="app card" data-testid="lol-app-game" data-just-ended={game?.justEnded ? "true" : "false"}>
    {#if game}
      <div class="card-head">
        <span class="t">{justEnded ? m.plugin_lol_card_summary() : m.plugin_lol_page_game()}</span>
        <span class="sub">{#if mine}{@render outcome(mine.win)} · {/if}{game.queue} · {duration(game.durationS)}</span>
        {#if canOpen}
          <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} data-testid="lol-card-open"
            onclick={() => void session.requestDisplayMode("fullscreen")}><Icon name="open" /></button>
        {/if}
      </div>
      {#if me}
        {@const k = kdaOf(me.kills, me.deaths, me.assists)}
        <div class="mine" data-testid="lol-card-me">
          <ChampIcon id={me.champion.id} icon={me.champion.icon} name={me.champion.short} size={32} level={me.level} />
          <div class="mine-text">
            <p><b>{me.champion.short}</b> <span class="num">{me.kills}/{me.deaths}/{me.assists} · KDA {one(k)}</span></p>
            <p class="muted num">
              {myDamageRank ? m.plugin_lol_card_damage_rank({ damage: num(me.damage), rank: myDamageRank }) : m.plugin_lol_damage_short({ damage: num(me.damage) })}
              · {m.plugin_lol_card_more_stats({ taken: num(me.taken), gold: num(me.gold), cs: one(me.cs / minutes) })}
            </p>
            <p class="gear-line">
              {#if me.augments.length}{#each me.augments as a (a.id)}<Augment augment={a} size={20} />{/each}{:else}<Perks perks={me.perks} size={20} sub={16} />{/if}
              <span class="gap"></span><Items items={me.items} size={20} />
            </p>
          </div>
        </div>
      {/if}
      {#each teams as t (t.teamId)}
        <div class="cteam {side(t)}">
          <span class="cteam-label"><span class="mark" class:diamond={side(t) === "enemy"}></span>{teamName(t)}
            <b class={t.win ? "good" : "bad"}>{t.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</b></span>
          <span class="cells">
            {#each t.players as p, i (p.puuid || i)}
              <span class="cell" class:me={p.me} title="{p.riotId} · {num(p.damage)}">
                <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={20} />
                <span class="cell-kda num">{p.kills}/{p.deaths}/{p.assists}</span>
                <span class="mini"><span style:width="{Math.round((p.damage / top) * 100)}%"></span></span>
              </span>
            {/each}
          </span>
        </div>
      {/each}
      <Footer data="" card onLink={session.openLink} />
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
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-bottom: var(--lol-sp-4);
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .back:hover {
    color: var(--lol-text);
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
    margin: 0 0 var(--lol-sp-5);
    font-size: var(--lol-fs-md);
    color: var(--lol-text-2);
  }
  .title .outcome {
    font-size: var(--lol-fs-lg);
  }
  .tag-ink {
    padding: 0 6px;
    border-radius: var(--lol-r-xs);
    background: var(--lol-ink-fill);
    color: var(--lol-ink-on);
    font-size: var(--lol-fs-xs);
    line-height: 18px;
    font-weight: var(--lol-fw-medium);
  }
  .sep {
    color: var(--lol-text-3);
  }
  .outcome {
    font-weight: 700;
  }
  .team + .team {
    margin-top: var(--lol-sp-5);
  }
  .wl {
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-medium);
  }
  .grid {
    display: grid;
    /* 每行是自己的 grid：列宽都写死（头像和技能 72、装备 6 + 1 格 160），表头和每一行才对得齐 */
    grid-template-columns: 72px minmax(7em, 1fr) 6.5em 10em 4.6em 4.6em 3em 160px;
    align-items: center;
    column-gap: 10px;
    padding: 0 12px 0 14px;
  }
  .cols {
    height: 26px;
    border-bottom: 1px solid var(--lol-line);
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    white-space: nowrap;
  }
  .row {
    min-height: var(--lol-row-list);
    padding-top: 4px;
    padding-bottom: 4px;
  }
  .row.augs {
    min-height: 52px;
  }
  .row + .row {
    border-top: 1px solid var(--lol-line);
  }
  .row.me {
    background: var(--lol-ally-soft);
  }
  .pics {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--lol-text-2);
  }
  .name:hover:not(:disabled) {
    color: var(--lol-text);
    text-decoration: underline;
    text-decoration-color: var(--lol-line-2);
  }
  .name:disabled {
    cursor: default;
  }
  .tagline {
    color: var(--lol-text-3);
  }
  .kda {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
    white-space: nowrap;
  }
  .kda b {
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .dmg {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .bar {
    flex: none;
    width: 64px;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--lol-bg-3);
  }
  .bar span,
  .mini span {
    display: block;
    height: 100%;
    background: var(--team);
  }
  .neutral .bar span,
  .neutral .mini span {
    background: var(--lol-text-3);
  }
  .r {
    text-align: right;
  }
  .muted {
    color: var(--lol-text-3);
  }
  .gear {
    display: grid;
    gap: 2px;
  }
  .aug-row {
    display: flex;
    gap: 2px;
  }
  .wide {
    display: contents;
  }
  @container (max-width: 879px) {
    .grid {
      grid-template-columns: 72px minmax(6em, 1fr) 6.5em 9em 160px;
    }
    .wide,
    .xcol {
      display: none;
    }
  }
  @container (max-width: 559px) {
    .grid {
      grid-template-columns: 40px minmax(0, 1fr) auto;
    }
    .cols {
      display: none;
    }
    .row .dmg {
      grid-column: 2 / 3;
    }
    .row .gear {
      grid-column: 1 / -1;
    }
  }
  /* 卡片 */
  .card .card-head .sub .outcome {
    font-size: var(--lol-fs-sm);
  }
  .mine {
    display: flex;
    gap: 10px;
    margin: 0 12px 8px;
  }
  .mine-text {
    min-width: 0;
  }
  .mine-text p {
    margin: 0;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-sm);
  }
  .mine-text p:first-child {
    font-size: var(--lol-fs-sm);
  }
  .gear-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    margin-top: 2px !important;
  }
  .gear-line .gap {
    width: 8px;
  }
  .cteam {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px;
  }
  .cteam + .cteam {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .cteam-label {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 5px;
    width: 4.6em;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
    white-space: nowrap;
  }
  .cells {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    flex: 1;
    gap: 4px;
    min-width: 0;
  }
  .cell {
    display: grid;
    justify-items: center;
    gap: 2px;
    min-width: 0;
    padding: 3px 2px;
    border-radius: var(--lol-r-xs);
  }
  .cell-kda {
    max-width: 100%;
    overflow: hidden;
    font-size: var(--lol-fs-2xs);
    line-height: var(--lol-lh-2xs);
    white-space: nowrap;
    color: var(--lol-text-2);
  }
  .cell.me {
    box-shadow: inset 0 0 0 1.5px var(--lol-text);
  }
  .mini {
    justify-self: stretch;
    height: 3px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--lol-bg-3);
  }
</style>
