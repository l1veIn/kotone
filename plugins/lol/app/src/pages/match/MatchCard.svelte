<script lang="ts">
  // “开局战绩”卡片（inline：聊天里、悬浮窗的小窗里；推送“开局战绩”和她调 `lol.scout` 时）。照 LeagueAkari 的对局页排，
  // 适度简化（ADR 043 D9，玩家：“窄的时候两队上下排，每人一行，点开看详情”）：
  // - 窄（< 640 px，小窗大约 360 px）：两队上下排，对面在上；每人一行（CardRow），点一行在下面展开详情；
  // - 宽（≥ 640 px）：LeagueAkari 用户习惯的样子，每队一行、每人一列（TeamGrid），每列带最近 5 局。
  // 隐藏了身份的人照样一行（只有英雄，写“隐藏了身份”）；只写事实，不写谁是突破口（ADR 043 D2）。
  // 选人阶段只有我方一块，标题写“这局 · 选人中”。最底下只有来源标签（客户端），没有风险提示（D8）。
  import Footer from "../../components/Footer.svelte";
  import Icon from "../../components/Icon.svelte";
  import { duration } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchData, MatchPlayer } from "../../types";
  import CardRow from "./CardRow.svelte";
  import { hiddenIn, partiesIn, partyOf, phaseOf, teamStats } from "./match";
  import TeamGrid from "./TeamGrid.svelte";

  let { match, canOpen, onOpen, onLink }: { match: MatchData; canOpen: boolean; onOpen: () => void; onLink?: (url: string) => void } = $props();

  /** 宽到能并排放 5 列（LeagueAkari 的排布）。 */
  const WIDE = 640;
  let width = $state(0);
  const wide = $derived(width >= WIDE);
  const phase = $derived(phaseOf(match));
  const champSelect = $derived(phase === "champ_select");
  const party = $derived(partyOf(match));
  const allyParties = $derived(partiesIn(match.allies, party));
  const sub = $derived(
    [match.mode, ...allyParties.slice(0, 1).map((g) => m.plugin_lol_card_premade({ count: g.size }))].filter(Boolean).join(" · "),
  );
  /** 展开的那一行（一次只展开一个人，小窗里不挤）。 */
  let open = $state<string | null>(null);
  const keyOf = (p: MatchPlayer, ally: boolean, i: number) => p.puuid || `${ally ? "a" : "e"}${i}`;
  const toggle = (key: string) => (open = open === key ? null : key);
</script>

{#snippet rows(ally: boolean, team: MatchPlayer[])}
  {@const st = teamStats(team)}
  {@const hidden = hiddenIn(team)}
  <div class={ally ? "ally" : "enemy"} data-testid={ally ? "lol-card-allies" : "lol-card-enemies"}>
    <div class="chead">
      <span class="label">
        <span class="mark" class:diamond={!ally}></span><b>{ally ? m.plugin_lol_allies() : m.plugin_lol_enemies()}</b>
        {#if st.known > 0}<span class="num">{m.plugin_lol_avg_short()} <b class={st.rateTone}>{st.rate}%</b></span>{/if}
        {#if hidden > 0}<span class="warn" data-testid="lol-team-hidden">{m.plugin_lol_hidden_count({ count: hidden })}</span>{/if}
      </span>
      <span class="c-rec">{m.plugin_lol_col_recent()}</span>
      <span class="c-kda">KDA</span>
    </div>
    {#each team as p, i (keyOf(p, ally, i))}
      {@const key = keyOf(p, ally, i)}
      <CardRow {p} party={party(p)} mode={match.mode} {champSelect} expanded={open === key} onToggle={() => toggle(key)} />
    {/each}
  </div>
{/snippet}

<div class="card" bind:clientWidth={width} data-testid="lol-match-card" data-phase={phase ?? ""} data-layout={wide ? "grid" : "rows"}>
  <div class="card-head">
    <span class="t">{champSelect ? m.plugin_lol_card_title_select() : m.plugin_lol_card_title()}</span>
    <span class="sub">{sub}</span>
    {#if canOpen}
      <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} data-testid="lol-card-open" onclick={onOpen}>
        <Icon name="open" />
      </button>
    {/if}
  </div>
  {#if phase === "post_match" && match.result}
    <p class="card-result num">
      <b class={match.result.win ? "good" : "bad"}>{match.result.win ? m.plugin_lol_victory() : m.plugin_lol_defeat()}</b>
      · {duration(match.result.durationS)}
    </p>
  {/if}
  {#if wide}
    <div class="grids">
      {#if !champSelect}<TeamGrid ally={false} team={match.enemies} {party} testid="lol-card-row" />{/if}
      <TeamGrid ally team={match.allies} {party} {champSelect} testid="lol-card-row" />
    </div>
  {:else}
    {#if !champSelect}{@render rows(false, match.enemies)}{/if}
    {@render rows(true, match.allies)}
  {/if}
  <Footer card sources={[{ id: "client" }]} extra={match.waiting > 0 ? m.plugin_lol_waiting_count({ count: match.waiting }) : ""} {onLink} />
</div>

<style>
  .chead {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 6.4em 2.4em 12px;
    align-items: center;
    column-gap: 6px;
    height: 26px;
    margin-top: 4px;
    padding: 0 10px 0 14px;
    /* 列宽的 em 和行（CardRow）用同一个字号算，列才对得齐；字本身是小号 */
    font-size: var(--lol-fs-sm);
    color: var(--lol-text-3);
    white-space: nowrap;
  }
  .chead > span {
    font-size: var(--lol-fs-xs);
  }
  @container (min-width: 460px) {
    .chead {
      grid-template-columns: minmax(0, 1fr) 6.4em 2.4em 3.8em 12px;
    }
  }
  .chead .label {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    overflow: hidden;
    color: var(--lol-text-2);
  }
  .chead .label b {
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text);
  }
  .chead .warn {
    color: var(--lol-warn);
  }
  .chead .c-rec,
  .chead .c-kda {
    text-align: right;
  }
  .grids {
    display: grid;
    gap: var(--lol-sp-4);
    padding: 0 12px;
  }
  .card-result {
    margin: 0 12px 2px;
    font-size: var(--lol-fs-sm);
    color: var(--lol-text-2);
  }
</style>
