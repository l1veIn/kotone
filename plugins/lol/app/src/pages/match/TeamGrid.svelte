<script lang="ts">
  // 宽的时候（≥ 640 px）一队：照 LeagueAkari 对局页的排布（大家用惯了，ADR 043 D9）——表头一行，下面每人一列并排
  // （5 列），一列从上到下：英雄头像 + 召唤师等级、英雄名、名字#编号、段位，近 20 局的胜率（大字）和胜负、KDA，
  // 标签，最近几局一局一行（英雄、胜负、K/D/A）。只学排布，视觉是我们的“记分纸”（docs/design/lol-app.md）：
  // 列顶 3 px 的队伍色条、开黑是字母小牌、胜负配字。隐藏了身份的人、电脑那一列只有英雄和一句说明。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Icon from "../../components/Icon.svelte";
  import RankBadge from "../../components/RankBadge.svelte";
  import Tags from "../../components/Tags.svelte";
  import { championIconPath } from "../../assets";
  import Pic from "../../components/Pic.svelte";
  import { kdaTone, one, pctOf, rateTone, splitRiotId } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchPlayer } from "../../types";
  import { hiddenIn, identityOf, pickOf, type PartyInfo } from "./match";
  import TeamHead from "./TeamHead.svelte";

  let {
    ally,
    team,
    party,
    champSelect = false,
    recent = 5,
    full = false,
    testid,
    onPlayer,
  }: {
    ally: boolean;
    team: MatchPlayer[];
    party: (p: MatchPlayer) => PartyInfo | null;
    champSelect?: boolean;
    /** 每人列几局（卡片 5，全屏 10）。 */
    recent?: number;
    full?: boolean;
    /** 每一列的 data-testid（卡片 `lol-card-row`，全屏 `lol-match-player`）。 */
    testid: string;
    onPlayer?: (p: MatchPlayer) => void;
  } = $props();

  const hidden = $derived(hiddenIn(team));
</script>

<section class="team {ally ? 'ally' : 'enemy'}" class:full aria-label={ally ? m.plugin_lol_allies() : m.plugin_lol_enemies()} data-testid={ally ? "lol-match-allies" : "lol-match-enemies"}>
  <TeamHead {ally} {team} {party} stats={!champSelect} />
  {#if hidden > 0}<p class="hidden-note" data-testid="lol-team-hidden">{m.plugin_lol_hidden_count({ count: hidden })}</p>{/if}
  <div class="grid" style:--cols={Math.max(team.length, 1)}>
    {#each team as p, i (p.puuid || `${ally ? "a" : "e"}${i}`)}
      {@const s = p.summary && p.summary.games > 0 ? p.summary : null}
      {@const identity = identityOf(p)}
      {@const pt = party(p)}
      {@const pick = champSelect ? pickOf(p) : "locked"}
      {@const [nm, tag] = splitRiotId(p.riotId || p.name)}
      {@const clickable = !!onPlayer && !!p.puuid && identity === "known"}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="col"
        class:me={p.me}
        class:clickable
        role={clickable ? "button" : undefined}
        tabindex={clickable ? 0 : undefined}
        title={clickable ? m.plugin_lol_open_player({ name: p.riotId || p.name }) : identity === "hidden" ? m.plugin_lol_identity_hidden_desc() : undefined}
        data-testid={testid}
        data-puuid={p.puuid}
        data-identity={identity}
        onclick={() => clickable && onPlayer?.(p)}
        onkeydown={(e) => {
          if (clickable && (e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            onPlayer?.(p);
          }
        }}
      >
        <div class="profile">
          <div class="head">
            {#if pick === "none"}
              <span class="ci s32 empty" aria-hidden="true"></span>
            {:else}
              <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={full ? 40 : 32} level={p.summonerLevel} />
            {/if}
            <span class="names">
              <span class="champ">{pick === "none" ? m.plugin_lol_not_picked() : p.champion.short}{#if pt}<span class="party" style:--party={pt.color} title={m.plugin_lol_party_title({ letter: pt.letter, count: pt.size })} data-testid="lol-match-party">{pt.letter}</span>{/if}</span>
              {#if identity === "known"}
                <span class="who" title={p.riotId || p.name}>{nm}{#if tag}<span class="tagline">#{tag}</span>{/if}</span>
              {:else if identity === "name_hidden"}
                <span class="who">{m.plugin_lol_name_hidden()}</span>
              {/if}
            </span>
          </div>
          <div class="marks">
            {#if identity === "hidden"}<span class="chip warn" data-testid="lol-match-identity">{m.plugin_lol_identity_hidden()}</span>{/if}
            {#if identity === "bot"}<span class="chip" data-testid="lol-match-identity">{m.plugin_lol_identity_bot()}</span>{/if}
            {#if p.me}<span class="chip me">{m.plugin_lol_me_tag()}</span>{/if}
            {#if p.hidden}<span class="chip warn" data-testid="lol-match-hidden"><Icon name="eyeOff" />{m.plugin_lol_hidden()}</span>{/if}
            {#if identity === "known" && !p.pending}<RankBadge ranked={p.ranked?.solo} legacy={p.rank} short />{/if}
          </div>
          {#if identity === "hidden"}
            <p class="why">{m.plugin_lol_identity_hidden_desc()}</p>
          {:else if identity !== "known"}
            <!-- 电脑、看不到名字：只有英雄 -->
          {:else if p.pending}
            <p class="why">{m.plugin_lol_pending_row()}</p>
            <span class="skel" style:width="70%"></span>
          {:else if s}
            <p class="big num"><b class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</b><span>{m.plugin_lol_wins_losses_tight({ wins: s.wins, losses: s.losses })}</span></p>
            <p class="line num">KDA <b class={kdaTone(s.kda)}>{one(s.kda)}</b> · {m.plugin_lol_avg_deaths()} <b>{one(s.deaths)}</b></p>
            {#if p.tags.length}<p class="tags"><Tags tags={p.tags} limit={2} summary={s} /></p>{/if}
          {:else}
            <p class="why">{p.error ? m.plugin_lol_lookup_failed({ error: p.error }) : m.plugin_lol_no_games()}</p>
          {/if}
        </div>
        {#if identity === "known" && !p.pending && s}
          <ol class="games" aria-label={m.plugin_lol_recent({ count: Math.min(recent, p.recent.length) })}>
            {#each p.recent.slice(0, recent) as g (g.gameId)}
              <li class:w={g.win} class:l={!g.win}>
                <Pic path={g.championIcon || championIconPath(g.championId)} name={g.win ? m.plugin_lol_win() : m.plugin_lol_loss()} size={18} crop />
                <span class="res">{g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}</span>
                <span class="num">{g.kills}/{g.deaths}/{g.assists}</span>
              </li>
            {/each}
          </ol>
        {/if}
      </div>
    {/each}
  </div>
</section>

<style>
  .profile {
    display: contents;
  }
  .hidden-note {
    margin: 0;
    padding: 4px 14px 0;
    font-size: var(--lol-fs-xs);
    color: var(--lol-warn);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: 6px;
    padding: 8px;
  }
  .col {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
    padding: 10px 8px 8px;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-md);
    background: var(--lol-bg);
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  /* 列顶的队伍色条（和窄时行左边的边条是同一个记号） */
  .col::before {
    content: "";
    position: absolute;
    left: -1px;
    right: -1px;
    top: -1px;
    height: 3px;
    border-radius: var(--lol-r-md) var(--lol-r-md) 0 0;
    background: var(--team);
  }
  .col.me {
    background: var(--lol-ally-soft);
  }
  .col.clickable {
    cursor: pointer;
  }
  .col.clickable:hover {
    border-color: var(--lol-line-2);
  }
  .col.clickable:focus-visible {
    outline: 2px solid var(--lol-ring);
    outline-offset: 1px;
  }
  .head {
    display: flex;
    gap: 6px;
    min-width: 0;
  }
  .names {
    display: grid;
    min-width: 0;
  }
  .champ {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text);
    white-space: nowrap;
  }
  .who {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tagline {
    color: var(--lol-text-3);
  }
  .marks {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    min-height: 18px;
  }
  .why {
    margin: 0;
    color: var(--lol-text-3);
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin: 2px 0 0;
  }
  .big b {
    font-size: var(--lol-fs-lg);
    line-height: var(--lol-lh-lg);
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text);
  }
  .line {
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .line b {
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 0;
  }
  .games {
    display: grid;
    gap: 2px;
    margin: 2px 0 0;
    padding: 0;
    list-style: none;
  }
  .games li {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 4px 0 2px;
    border-radius: var(--lol-r-xs);
    white-space: nowrap;
    overflow: hidden;
  }
  .games li.w {
    background: var(--lol-win-soft);
  }
  .games li.l {
    background: var(--lol-loss-soft);
  }
  .games .res {
    flex: none;
    font-weight: var(--lol-fw-medium);
  }
  .games li.w .res {
    color: var(--lol-win);
  }
  .games li.l .res {
    color: var(--lol-loss);
  }
  .games li.l :global(img) {
    opacity: 0.6;
  }

  .full .grid {
    padding: 0;
    gap: 0;
  }
  .full .col {
    display: grid;
    grid-row: span 2;
    grid-template-rows: subgrid;
    row-gap: 0;
    padding: var(--lol-sp-5);
    border: 0;
    border-radius: 0;
    background: transparent;
  }
  .full .col + .col {
    border-inline-start: 1px solid var(--lol-line);
  }
  .full .col::before {
    content: none;
  }
  .full .col.clickable:is(:hover, :focus-visible) {
    background: var(--lol-hover);
  }
  .full .col.clickable:focus-visible {
    outline-offset: -2px;
  }
  .full .profile {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--lol-sp-2);
    padding-block-end: var(--lol-sp-4);
  }
  .full .head {
    align-items: center;
    gap: var(--lol-sp-4);
    max-width: 100%;
  }
  .full .champ {
    font-size: var(--lol-fs-md);
  }
  .full .big {
    flex-wrap: wrap;
    margin-block-start: var(--lol-sp-2);
  }
  .full .big b {
    font-size: var(--lol-fs-xl);
    line-height: var(--lol-lh-xl);
  }
  .full .line,
  .full .champ {
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .full .games {
    grid-row: 2;
    align-self: start;
    margin: 0;
    padding-block-start: var(--lol-sp-3);
    border-block-start: 1px solid var(--lol-line);
    gap: 0 var(--lol-sp-5);
  }
  .full .games:empty {
    display: none;
  }
  .full .games li {
    display: grid;
    grid-template-columns: auto auto 1fr;
    min-height: 24px;
    height: auto;
    padding: 0;
    background: transparent;
  }
  .full .games li .num {
    text-align: end;
    color: var(--lol-text-2);
  }
  @container (max-width: 800px) {
    .full .col {
      padding-inline: var(--lol-sp-3);
    }
    .full .head {
      gap: var(--lol-sp-2);
    }
  }
  /* Five columns, each with room for two complete game results. */
  @container (min-width: 1184px) {
    .full .games {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
