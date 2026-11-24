<script lang="ts">
  // “开局战绩”卡片（inline：聊天卡片、自动卡片；样张第二张图）。一人一行 28 px，列固定宽，列头和行用同一套列：
  // 头像 24 · 英雄 · 名字 · 段位 · 近 20 局 · KDA · 标签（最多 1 个）。对面在上；突破口那一行墨线圈住，最下面写原因。
  // 选人阶段只有我方一块，标题写“这局 · 选人中”。窄于 400 时去掉标签列和段位字，只留徽章。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Icon from "../../components/Icon.svelte";
  import RankBadge from "../../components/RankBadge.svelte";
  import Tags from "../../components/Tags.svelte";
  import Footer from "../../components/Footer.svelte";
  import { duration, kdaTone, one, pctOf, rateTone, splitRiotId } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchData, MatchPlayer } from "../../types";
  import { partiesIn, partyOf, phaseOf, pickOf, teamStats } from "./match";

  let { match, canOpen, onOpen, onLink }: { match: MatchData; canOpen: boolean; onOpen: () => void; onLink?: (url: string) => void } = $props();

  const phase = $derived(phaseOf(match));
  const champSelect = $derived(phase === "champ_select");
  const party = $derived(partyOf(match));
  const allyParties = $derived(partiesIn(match.allies, party));
  const weak = $derived(match.enemies.filter((p) => p.weak).slice(0, 2));
  const sub = $derived(
    [match.mode, ...allyParties.slice(0, 1).map((g) => m.plugin_lol_card_premade({ count: g.size }))].filter(Boolean).join(" · "),
  );
</script>

{#snippet block(ally: boolean, team: MatchPlayer[])}
  {@const st = teamStats(team)}
  <div class={ally ? "ally" : "enemy"} data-testid={ally ? "lol-card-allies" : "lol-card-enemies"}>
    <div class="cgrid chead">
      <span class="label">
        <span class="mark" class:diamond={!ally}></span><b>{ally ? m.plugin_lol_allies() : m.plugin_lol_enemies()}</b>
        {#if st.known > 0}<span class="num">{m.plugin_lol_avg_short()} <b class={st.rateTone}>{st.rate}%</b></span>{/if}
      </span>
      <span class="c-rank">{m.plugin_lol_col_rank()}</span>
      <span class="c-rec">{m.plugin_lol_col_recent()}</span>
      <span class="c-kda">KDA</span>
      <span class="c-tag"></span>
    </div>
    {#each team as p, i (p.puuid || `${ally ? "a" : "e"}${i}`)}
      {@const s = p.summary && p.summary.games > 0 ? p.summary : null}
      {@const pt = party(p)}
      {@const pick = champSelect ? pickOf(p) : "locked"}
      {@const [nm] = splitRiotId(p.riotId || p.name)}
      <div class="cgrid cr stripe" class:me={p.me} class:weak={!!p.weak} title={p.riotId || p.name} data-testid="lol-card-row">
        {#if pick === "none"}
          <span class="ci s24 empty" aria-hidden="true"></span>
          <span class="champ muted">{m.plugin_lol_not_picked()}</span>
        {:else}
          <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={24} />
          <span class="champ">{p.champion.short}</span>
        {/if}
        <span class="who">
          <span class="nm">{nm || (p.puuid ? "" : m.plugin_lol_name_hidden())}</span>
          {#if pt}<span class="party" style:--party={pt.color}>{pt.letter}</span>{/if}
          {#if p.hidden}<span class="eye" title={m.plugin_lol_hidden()}><Icon name="eyeOff" /></span>{/if}
        </span>
        <span class="rk">{#if !p.pending}<RankBadge ranked={p.ranked?.solo} legacy={p.rank} short />{/if}</span>
        {#if !p.puuid && !p.riotId && !p.name}
          <span class="span2"></span>
        {:else if p.pending}
          <span class="span2">{m.plugin_lol_pending_row()}</span>
        {:else if s}
          <span class="rec num">{m.plugin_lol_wins_losses_tight({ wins: s.wins, losses: s.losses })} <span class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</span></span>
          <span class="kda num {kdaTone(s.kda)}">{one(s.kda)}</span>
        {:else}
          <span class="span2">{p.error ? m.plugin_lol_lookup_failed_short() : m.plugin_lol_no_games_short()}</span>
        {/if}
        <span class="tags"><Tags tags={p.tags} limit={1} summary={s} /></span>
      </div>
    {/each}
  </div>
{/snippet}

<div class="card" data-testid="lol-match-card" data-phase={phase ?? ""}>
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
  {#if !champSelect}{@render block(false, match.enemies)}{/if}
  {@render block(true, match.allies)}
  {#each weak as p (p.puuid)}
    <div class="card-why" data-testid="lol-card-weak"><Icon name="target" /><span><b>{p.champion.short}</b>{m.plugin_lol_card_weak_why({ why: (p.weak ?? []).join(m.plugin_lol_list_sep()) })}</span></div>
  {/each}
  <Footer data="" card extra={match.waiting > 0 ? m.plugin_lol_waiting_count({ count: match.waiting }) : ""} onLink={onLink} />
</div>

<style>
  .cgrid {
    display: grid;
    grid-template-columns: 24px 4.4em minmax(0, 1fr) 4em 6.2em 2.2em 3.6em;
    align-items: center;
    column-gap: 6px;
    padding: 0 12px 0 14px;
  }
  .chead {
    height: 26px;
    margin-top: 4px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    white-space: nowrap;
  }
  .chead .label {
    grid-column: 1 / 4;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    color: var(--lol-text-2);
  }
  .chead .label b {
    font-weight: var(--lol-fw-strong);
    color: var(--lol-text);
  }
  .chead .c-rec,
  .chead .c-kda {
    text-align: right;
  }
  .cr {
    height: var(--lol-row-card);
    white-space: nowrap;
  }
  .cr + .cr {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .cr.me {
    background: var(--lol-ally-soft);
  }
  .cr.weak {
    outline: 1.5px solid var(--lol-text);
    outline-offset: -2px;
    border-radius: 6px;
  }
  .champ {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-strong);
  }
  .champ.muted {
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-3);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .who .nm {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .eye {
    display: inline-flex;
    color: var(--lol-warn);
  }
  .eye :global(.ico) {
    width: 13px;
    height: 13px;
  }
  .rk {
    min-width: 0;
    overflow: hidden;
  }
  .rec {
    text-align: right;
    color: var(--lol-text-2);
  }
  .kda {
    text-align: right;
  }
  .span2 {
    grid-column: span 2;
    text-align: right;
    color: var(--lol-text-3);
  }
  .tags {
    display: flex;
    justify-content: flex-end;
    min-width: 0;
    overflow: hidden;
  }
  .tags :global(.chip) {
    display: inline-block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-why {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px 12px 0;
    padding: 6px 8px;
    border-radius: var(--lol-r-md);
    background: var(--lol-bg-2);
  }
  .card-result {
    margin: 0 12px 2px;
    font-size: var(--lol-fs-sm);
    color: var(--lol-text-2);
  }
  @container (max-width: 400px) {
    .cgrid {
      grid-template-columns: 24px 4.4em minmax(0, 1fr) 16px 6.2em 2.2em;
    }
    .tags,
    .c-tag {
      display: none !important;
    }
    .chead .c-rank {
      visibility: hidden;
    }
    .rk :global(.rt) {
      display: none;
    }
  }
</style>
