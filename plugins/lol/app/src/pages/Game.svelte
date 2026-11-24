<script lang="ts">
  // 单局详情（`ui://kotone/lol/game`，`lol.game` 的页面，只给页面用）。第一段先做能用的样子：两队各一块，
  // 每人一行（英雄、技能、符文、名字、KDA、伤害条、承伤、经济、补刀、装备、海克斯）；点名字看那个人的战绩。
  // 第二段照 docs/design/lol-app.md 第三节 3 做完整（“刚结束”头部、我那一行、卡片“这局总结”）。
  import { onMount } from "svelte";
  import Augment from "../components/Augment.svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import Items from "../components/Items.svelte";
  import Perks from "../components/Perks.svelte";
  import Spells from "../components/Spells.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { duration, kdaOf, kdaTone, num, one, splitRiotId, when } from "../format";
  import { resultStatus, TOOL_GAME, type DisplayMode, type Session, type ToolResult } from "../host";
  import { m } from "../i18n.svelte";
  import { isGameData, type GameData } from "../types";

  let {
    session,
    initial,
    display,
    gameId = 0,
    onTab,
    onPlayer,
    onBack,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    gameId?: number;
    onTab: (page: PageId) => void;
    onPlayer: (puuid: string) => void;
    onBack?: () => void;
  } = $props();

  let game = $state<GameData | null>(null);
  let status = $state("");
  let error = $state("");
  let loading = $state(false);

  function show(result: ToolResult) {
    if (isGameData(result.structuredContent)) {
      game = result.structuredContent;
      status = "";
    } else {
      status = resultStatus(result).text;
    }
  }

  async function load() {
    const id = gameId || Number(session.input()?.gameId ?? 0);
    if (!id) return;
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

  const top = $derived(Math.max(1, ...(game?.teams.flatMap((t) => t.players.map((p) => p.damage)) ?? [1])));
</script>

<div class="app" data-testid="lol-app-game">
  {#if display !== "inline"}
    <TopBar current="player" {onTab} onRefresh={() => void load()} {loading} />
  {/if}
  <main class="page">
    {#if onBack}
      <button type="button" class="back" onclick={onBack}><Icon name="back" />{m.plugin_lol_page_player()}</button>
    {/if}
    {#if error}<p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_failed({ error })}</p>{/if}
    {#if status}<p class="banner" data-testid="lol-app-status"><Icon name="info" />{status}</p>{/if}
    {#if game}
      <p class="title num">{game.queue} · {duration(game.durationS)} · {when(game.createdMs)}</p>
      {#each game.teams as team (team.teamId)}
        {@const kills = team.players.reduce((a, p) => a + p.kills, 0)}
        <section class="team neutral">
          <div class="team-head">
            <span class="label"><span class="mark"></span><span class={team.win ? "good" : "bad"}>{team.win ? m.plugin_lol_team_win() : m.plugin_lol_team_loss()}</span></span>
            <span class="stats num">{m.plugin_lol_team_kills({ kills })}</span>
          </div>
          {#each team.players as p, i (p.puuid || i)}
            {@const k = kdaOf(p.kills, p.deaths, p.assists)}
            {@const [nm, tag] = splitRiotId(p.riotId)}
            <div class="row stripe">
              <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={32} level={p.level} />
              <Spells spells={p.spells} size={15} />
              <Perks perks={p.perks} size={15} sub={12} />
              <button type="button" class="name" disabled={!p.puuid} onclick={() => onPlayer(p.puuid)} title={p.riotId}>
                {nm}{#if tag}<span class="tagline">#{tag}</span>{/if}
              </button>
              <span class="kda num"><b>{p.kills}/{p.deaths}/{p.assists}</b> <span class={kdaTone(k)}>{one(k)}</span></span>
              <span class="dmg num" title={m.plugin_lol_game_more({ taken: num(p.taken), gold: num(p.gold), cs: p.cs })}>
                <span class="bar"><span style:width="{Math.round((p.damage / top) * 100)}%"></span></span>{num(p.damage)}
              </span>
              <span class="gear">
                <Items items={p.items} size={20} />
                {#if p.augments.length}<span class="augs">{#each p.augments as a (a.id)}<Augment augment={a} size={18} />{/each}</span>{/if}
              </span>
            </div>
          {/each}
        </section>
      {/each}
    {:else if loading}
      <p class="banner">{m.plugin_lol_loading()}</p>
    {/if}
    {#if display !== "inline"}<Footer data={m.plugin_lol_foot_client()} onLink={session.openLink} />{/if}
  </main>
</div>

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
  .title {
    margin: 0 0 var(--lol-sp-4);
    font-size: var(--lol-fs-md);
    color: var(--lol-text-2);
  }
  .team + .team {
    margin-top: var(--lol-sp-5);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: var(--lol-row-list);
    padding: 4px 12px 4px 14px;
  }
  .row + .row {
    border-top: 1px solid var(--lol-line);
  }
  .name {
    width: 9em;
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
  .name:disabled {
    cursor: default;
  }
  .tagline {
    color: var(--lol-text-3);
  }
  .kda {
    width: 7em;
    flex: none;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .kda b {
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .dmg {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    width: 130px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .bar {
    width: 60px;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--lol-bg-3);
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--lol-text-3);
  }
  .gear {
    display: grid;
    gap: 2px;
    margin-left: auto;
  }
  .augs {
    display: flex;
    gap: 2px;
  }
  @container (max-width: 719px) {
    .row :global(.perks),
    .dmg {
      display: none;
    }
  }
</style>
