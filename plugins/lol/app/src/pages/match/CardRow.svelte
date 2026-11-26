<script lang="ts">
  // “开局战绩”卡片窄的时候（悬浮窗的小窗、聊天里，< 640 px）的一个人：一行 32 px，点一下在下面展开详情
  // （照 LeagueAkari 对局页一人一张卡片的字段，适度简化：一行只放英雄、名字、近 20 局、KDA，别的点开看）。
  //   一行：头像 24 · 英雄 · 名字（开黑字母、隐藏战绩的眼睛；隐藏了身份 / 电脑写明）· 近 20 局胜负和胜率 · KDA · 标签（宽一点才有）· ▾
  //   展开：段位（单双排、灵活组排）、召唤师等级；胜负、KDA、场均死、这个模式；最近 10 局的英雄；全部标签；“看他的战绩”（全屏里才有）。
  // 隐藏了身份的人、电脑、看不到名字的人只有英雄，不展开（ADR 043 D9）。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Icon from "../../components/Icon.svelte";
  import RankBadge from "../../components/RankBadge.svelte";
  import Recent from "../../components/Recent.svelte";
  import Tags from "../../components/Tags.svelte";
  import { kdaTone, num, one, pctOf, rateTone, splitRiotId } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchPlayer } from "../../types";
  import { identityOf, pickOf, type PartyInfo } from "./match";

  let {
    p,
    party,
    mode,
    champSelect = false,
    expanded = false,
    onToggle,
    onPlayer,
  }: {
    p: MatchPlayer;
    party: PartyInfo | null;
    mode: string;
    champSelect?: boolean;
    expanded?: boolean;
    onToggle?: () => void;
    /** 全屏里点“看他的战绩”；卡片里没有（卡片是快照）。 */
    onPlayer?: (p: MatchPlayer) => void;
  } = $props();

  const s = $derived(p.summary && p.summary.games > 0 ? p.summary : null);
  const identity = $derived(identityOf(p));
  const pick = $derived(champSelect ? pickOf(p) : "locked");
  const [nm] = $derived(splitRiotId(p.riotId || p.name));
  /** 有东西可展开：查到了（或者查不到也写原因）。 */
  const expandable = $derived(identity === "known" && !p.pending && !!onToggle);
</script>

{#snippet cells()}
  {#if pick === "none"}
    <span class="ci s24 empty" aria-hidden="true"></span>
    <span class="champ muted">{m.plugin_lol_not_picked()}</span>
  {:else}
    <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={24} />
    <span class="champ">{p.champion.short}</span>
  {/if}
  <span class="who">
    {#if identity === "hidden"}
      <span class="chip warn" title={m.plugin_lol_identity_hidden_desc()} data-testid="lol-card-identity">{m.plugin_lol_identity_hidden()}</span>
    {:else if identity === "bot"}
      <span class="chip" data-testid="lol-card-identity">{m.plugin_lol_identity_bot()}</span>
    {:else if identity === "name_hidden"}
      <span class="nm muted">{m.plugin_lol_name_hidden()}</span>
    {:else}
      <span class="nm">{nm}</span>
    {/if}
    {#if party}<span class="party" style:--party={party.color} title={m.plugin_lol_party_title({ letter: party.letter, count: party.size })}>{party.letter}</span>{/if}
    {#if p.me}<span class="chip me">{m.plugin_lol_me_tag()}</span>{/if}
    {#if p.hidden}<span class="eye" title={m.plugin_lol_hidden()}><Icon name="eyeOff" /></span>{/if}
  </span>
  {#if identity !== "known"}
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
  <span class="chev" aria-hidden="true">{#if expandable}<svg viewBox="0 0 12 12" class:up={expanded}><path d="M3 4.5 6 7.5 9 4.5" /></svg>{/if}</span>
{/snippet}

<div class="person" class:me={p.me} class:open={expanded} data-testid="lol-card-player" data-identity={identity}>
  {#if expandable}
    <button
      type="button"
      class="cgrid cr stripe"
      aria-expanded={expanded}
      title={expanded ? undefined : m.plugin_lol_row_expand()}
      data-testid="lol-card-row"
      data-identity={identity}
      onclick={onToggle}
    >
      {@render cells()}
    </button>
  {:else}
    <div class="cgrid cr stripe" title={identity === "hidden" ? m.plugin_lol_identity_hidden_desc() : p.riotId || p.name} data-testid="lol-card-row" data-identity={identity}>
      {@render cells()}
    </div>
  {/if}
  {#if expanded && expandable}
    <div class="detail" data-testid="lol-card-detail">
      <p class="d1">
        <RankBadge ranked={p.ranked?.solo} legacy={p.rank} />
        {#if p.ranked?.flex}<span class="muted">{m.plugin_lol_queue_flex()}</span><RankBadge ranked={p.ranked.flex} short />{/if}
        {#if typeof p.summonerLevel === "number" && p.summonerLevel > 0}<span class="muted num">{m.plugin_lol_level_short({ level: p.summonerLevel })}</span>{/if}
      </p>
      {#if s}
        <p class="d2 num">
          <span>{m.plugin_lol_wins_losses({ wins: s.wins, losses: s.losses })} <b class={rateTone(s.wins, s.games)}>{pctOf(s.wins, s.games)}%</b></span>
          <span>KDA <b class={kdaTone(s.kda)}>{one(s.kda)}</b></span>
          <span>{m.plugin_lol_avg_deaths()} <b>{one(s.deaths)}</b></span>
          {#if s.modeGames > 0}
            <span>{m.plugin_lol_mode_line({ mode, games: s.modeGames, rate: pctOf(s.modeWins, s.modeGames), dpm: num(s.modeDpm) })}</span>
          {/if}
        </p>
        <p class="d3"><Recent games={p.recent.slice(0, 10)} /><span class="all-tags"><Tags tags={p.tags} summary={s} /></span></p>
      {:else if p.error}
        <p class="d2">{m.plugin_lol_lookup_failed({ error: p.error })}</p>
      {:else}
        <p class="d2">{m.plugin_lol_no_games()}</p>
      {/if}
      {#if onPlayer && p.puuid}
        <button type="button" class="btn" data-testid="lol-card-open-player" onclick={() => onPlayer?.(p)}>{m.plugin_lol_open_record()}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cgrid {
    display: grid;
    grid-template-columns: 24px 4.6em minmax(0, 1fr) 6.4em 2.4em 12px;
    align-items: center;
    column-gap: 6px;
    width: 100%;
    padding: 0 10px 0 14px;
  }
  .cr {
    height: 32px;
    border: 0;
    background: transparent;
    text-align: left;
    white-space: nowrap;
  }
  button.cr {
    cursor: pointer;
  }
  button.cr:hover {
    background: var(--lol-hover);
  }
  button.cr:focus-visible {
    outline: 2px solid var(--lol-ring);
    outline-offset: -2px;
  }
  .person + :global(.person) {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .person.me {
    background: var(--lol-ally-soft);
  }
  .person.open {
    background: var(--lol-bg-2);
  }
  .person.me.open {
    background: var(--lol-ally-soft);
  }
  .champ {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-strong);
  }
  .muted {
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
  /* 一行最多一个标签，窄的时候不放（展开后看全部） */
  .tags {
    display: none;
  }
  .chev {
    display: inline-flex;
    justify-content: center;
    color: var(--lol-text-3);
  }
  .chev svg {
    width: 10px;
    height: 10px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform var(--lol-dur-fast) var(--lol-ease);
  }
  .chev svg.up {
    transform: rotate(180deg);
  }
  @container (min-width: 460px) {
    .cgrid {
      grid-template-columns: 24px 4.6em minmax(0, 1fr) 6.4em 2.4em 3.8em 12px;
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
  }
  .detail {
    display: grid;
    gap: 6px;
    padding: 4px 12px 10px 44px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .detail p {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 10px;
    margin: 0;
  }
  .detail b {
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text);
  }
  .all-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .detail .btn {
    justify-self: start;
  }
  @media (prefers-reduced-motion: reduce) {
    .chev svg {
      transition: none;
    }
  }
</style>
