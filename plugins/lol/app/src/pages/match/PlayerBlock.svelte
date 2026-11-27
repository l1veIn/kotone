<script lang="ts">
  // “这局”全屏的一个人，三组信息随行宽换行：
  //   英雄头像 40（右下召唤师等级）+ 两个召唤师技能竖排 ·
  //   第一行：英雄名（选人时加分路）· 名字#编号 · 开黑字母 · 我 · 隐藏战绩 ……右边段位徽章 + 字
  //   第二行：近 20 局胜负 · 胜率 · KDA · 场均死 · 这个模式的局数、胜率、分均伤害
  //   第三行：最近 10 局的英雄小头像 · 标签
  //   隐藏了身份的人、电脑（ADR 043 D9）：只有第一行（英雄 + “隐藏了身份” / “电脑”），点不开。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Icon from "../../components/Icon.svelte";
  import LaneIcon from "../../components/LaneIcon.svelte";
  import RankBadge from "../../components/RankBadge.svelte";
  import Recent from "../../components/Recent.svelte";
  import Spells from "../../components/Spells.svelte";
  import Tags from "../../components/Tags.svelte";
  import { kdaTone, laneName, num, one, pctOf, rateTone, splitRiotId } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchPlayer } from "../../types";
  import { identityOf, pickOf, type PartyInfo } from "./match";

  let {
    p,
    party,
    mode,
    champSelect = false,
    onOpen,
  }: {
    p: MatchPlayer;
    party: PartyInfo | null;
    mode: string;
    champSelect?: boolean;
    onOpen?: (p: MatchPlayer) => void;
  } = $props();

  const s = $derived(p.summary && p.summary.games > 0 ? p.summary : null);
  const pick = $derived(champSelect ? pickOf(p) : "locked");
  const [nm, tag] = $derived(splitRiotId(p.riotId || p.name));
  const identity = $derived(identityOf(p));
  /** 只有英雄的人：隐藏了身份、电脑、排位选人时看不到名字的队友。 */
  const anonymous = $derived(identity !== "known");
  const clickable = $derived(!!p.puuid && !!onOpen && !anonymous);
  const recent = $derived(p.recent.slice(0, 10));

  function open() {
    if (clickable) onOpen?.(p);
  }
  function key(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      open();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="p"
  class:me={p.me}
  class:clickable
  role={clickable ? "button" : undefined}
  tabindex={clickable ? 0 : undefined}
  title={clickable ? m.plugin_lol_open_player({ name: p.riotId || p.name }) : undefined}
  data-testid="lol-match-player"
  data-puuid={p.puuid}
  data-identity={identity}
  data-pick={pick}
  onclick={open}
  onkeydown={key}
>
  <div class="pics">
    {#if pick === "none"}
      <span class="ci s40 empty" aria-hidden="true"></span>
    {:else}
      <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} level={p.summonerLevel} cls={pick === "intent" ? "intent" : ""} />
    {/if}
    <Spells spells={p.spells} />
  </div>
  <div class="body">
    <div class="l1">
      {#if pick === "none"}
        <span class="champ muted">{m.plugin_lol_not_picked()}</span>
      {:else}
        <span class="champ">{p.champion.short}</span>
        {#if pick === "intent"}<span class="intent-label">{m.plugin_lol_pick_intent()}</span>{/if}
      {/if}
      {#if champSelect && p.position}
        <span class="lane"><LaneIcon lane={p.position} />{laneName(p.position)}</span>
      {/if}
      {#if identity === "hidden"}
        <span class="chip warn" title={m.plugin_lol_identity_hidden_desc()} data-testid="lol-match-identity">{m.plugin_lol_identity_hidden()}</span>
      {:else if identity === "bot"}
        <span class="chip" data-testid="lol-match-identity">{m.plugin_lol_identity_bot()}</span>
      {:else if anonymous}
        <span class="who">{m.plugin_lol_name_hidden()}</span>
      {:else}
        <span class="who" title={p.riotId || p.name}>{nm}{#if tag}<span class="tagline">#{tag}</span>{/if}</span>
      {/if}
      {#if party}
        <span class="party" style:--party={party.color} title={m.plugin_lol_party_title({ letter: party.letter, count: party.size })} data-testid="lol-match-party">{party.letter}</span>
      {/if}
      {#if p.me}<span class="chip me">{m.plugin_lol_me_tag()}</span>{/if}
      {#if p.hidden}<span class="chip warn" data-testid="lol-match-hidden"><Icon name="eyeOff" />{m.plugin_lol_hidden()}</span>{/if}
      {#if !p.pending && !anonymous}
        <span class="end"><RankBadge ranked={p.ranked?.solo} legacy={p.rank} /></span>
      {/if}
    </div>
    {#if identity === "hidden"}
      <div class="l2">{m.plugin_lol_identity_hidden_desc()}</div>
    {:else if anonymous}
      <!-- 名字看不到、电脑：没有第二、三行 -->
    {:else if p.pending}
      <div class="l2">{m.plugin_lol_pending_row()} <span class="skel" style:width="72px"></span><span class="skel" style:width="48px"></span></div>
    {:else if s}
      <div class="l2 num">
        <span class="v">{m.plugin_lol_wins_losses({ wins: s.wins, losses: s.losses })}</span>
        <span class="v {rateTone(s.wins, s.games)}">{pctOf(s.wins, s.games)}%</span>
        <span>KDA <span class="v {kdaTone(s.kda)}">{one(s.kda)}</span></span>
        <span>{m.plugin_lol_avg_deaths()} <span class="v">{one(s.deaths)}</span></span>
        {#if s.modeGames > 0}
          <span>{m.plugin_lol_mode_line({ mode, games: s.modeGames, rate: pctOf(s.modeWins, s.modeGames), dpm: num(s.modeDpm) })}</span>
        {/if}
      </div>
      <div class="l3">
        <Recent games={recent} />
        <span class="tags"><Tags tags={p.tags} summary={s} /></span>
      </div>
    {:else if p.error}
      <div class="l2">{m.plugin_lol_lookup_failed({ error: p.error })}</div>
    {:else}
      <div class="l2">{m.plugin_lol_no_games()}</div>
    {/if}
  </div>
</div>

<style>
  .p {
    container-type: inline-size;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--lol-sp-2) var(--lol-sp-5);
    align-content: center;
    padding: var(--lol-sp-5);
  }
  .p.clickable {
    cursor: pointer;
  }
  .p.clickable:hover {
    background: var(--lol-hover);
  }
  .p.clickable:focus-visible {
    outline: 2px solid var(--lol-ring);
    outline-offset: -2px;
  }
  .p.me {
    background: var(--lol-ally-soft);
  }
  .pics {
    display: flex;
    grid-row: 1 / span 3;
    align-self: start;
    gap: var(--lol-sp-1);
  }
  .pics :global(.intent) {
    opacity: 0.5;
    box-shadow: 0 0 0 1px var(--lol-text-3);
    outline: 1px dashed var(--lol-text-2);
    outline-offset: -1px;
  }
  .body {
    display: contents;
  }
  .l1, .l2, .l3 {
    grid-column: 2;
    min-inline-size: 0;
  }
  .l1 {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    align-content: center;
    gap: var(--lol-sp-1) var(--lol-sp-3);
  }
  .champ {
    overflow-wrap: anywhere;
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-strong);
  }
  .champ.muted {
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-3);
  }
  .intent-label {
    flex: none;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
  }
  .lane {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 3px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .who {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--lol-text-2);
  }
  .tagline {
    color: var(--lol-text-3);
  }
  .end {
    display: inline-flex;
    flex: none;
    margin-inline-start: auto;
  }
  .l2 {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0 var(--lol-sp-4);
    overflow-wrap: anywhere;
    color: var(--lol-text-2);
  }
  .l2 .v {
    color: var(--lol-text);
  }
  .l3 {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--lol-sp-2) var(--lol-sp-4);
  }
  .l3 :global(.recent) {
    flex-wrap: wrap;
    max-inline-size: 100%;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--lol-sp-2);
    min-inline-size: 0;
  }
  .tags :global(.chip) {
    max-inline-size: 100%;
    height: auto;
    min-block-size: var(--lol-lh-sm);
    white-space: normal;
    overflow-wrap: anywhere;
  }
  @container (max-width: 400px) {
    .pics {
      grid-row: 1;
    }
    .l2, .l3 {
      grid-column: 1 / -1;
    }
    .l2 {
      margin-block-start: var(--lol-sp-1);
    }
  }
</style>
