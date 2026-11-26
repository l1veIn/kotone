<script lang="ts">
  // 峡谷选人卡片随着进度长出来的几块（ADR 043 B 轮；后端 plugins/lol/src/pushes/draft.rs、comp.rs）：
  // 可以禁的（预选后、禁用结束前）、你的英雄池（轮到你选）、对线（锁定以后）、阵容的数（我方 3 个人选了以后）；
  // 选英雄卡片分两处用它：前三块在“我的英雄”上面，阵容在最后。
  // 只写事实和出处：胜率、强度、数出来的个数；不说谁该选什么（讨论记录第七节）。每块右上角一个来源标签（ADR 043 D8）。
  // 约 360 px（悬浮窗的小窗）也看得清：一个英雄一行，事实写在右边、放不下就省略号。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import SourceTag from "../../components/SourceTag.svelte";
  import { rate } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { BansBlock, CompBlock, CompFlag, CompSide, MatchupBlock, PoolBlock } from "./types";

  let {
    bans = null,
    comp = null,
    pool = null,
    matchup = null,
    onLink,
  }: {
    bans?: BansBlock | null;
    comp?: CompBlock | null;
    pool?: PoolBlock | null;
    matchup?: MatchupBlock | null;
    onLink: (url: string) => void;
  } = $props();

  function flag(f: CompFlag): string {
    switch (f) {
      case "all_physical":
        return m.plugin_lol_cs_comp_flag_all_physical();
      case "all_magic":
        return m.plugin_lol_cs_comp_flag_all_magic();
      case "no_frontline":
        return m.plugin_lol_cs_comp_flag_no_frontline();
      default:
        return m.plugin_lol_cs_comp_flag_no_cc();
    }
  }
  function style(s: string): string {
    switch (s.toLowerCase()) {
      case "even":
        return m.plugin_lol_cs_matchup_style_even();
      case "aggressive":
        return m.plugin_lol_cs_matchup_style_aggressive();
      case "passive":
      case "defensive":
        return m.plugin_lol_cs_matchup_style_passive();
      default:
        return s;
    }
  }
  const side = (who: "me" | "opponent" | null, block: MatchupBlock) =>
    who === "me" ? m.plugin_lol_me() : who === "opponent" ? block.opponent.short : "—";
</script>

{#snippet sideRow(label: string, s: CompSide, testid: string)}
  <div class="comp-side" data-testid={testid}>
    <span class="k">{label}</span>
    <span class="v num">{m.plugin_lol_cs_comp_damage({ physical: s.physical, magic: s.magic, mixed: s.mixed })}</span>
    <span class="v num">{m.plugin_lol_cs_comp_body({ frontline: s.frontline, cc: s.cc, ranged: s.ranged })}</span>
    {#if s.flags.length}
      <span class="flags">{#each s.flags as f (f)}<span class="chip warn" data-testid="lol-cs-comp-flag" data-flag={f}>{flag(f)}</span>{/each}</span>
    {/if}
  </div>
{/snippet}

{#if bans}
  <section class="blk" data-testid="lol-cs-bans">
    <header class="bh">
      <span class="bt">{m.plugin_lol_cs_bans_title()}</span>
      <SourceTag id={bans.source} {onLink} />
    </header>
    <p class="hint">{bans.against
      ? m.plugin_lol_cs_bans_hint_with({ lane: bans.lane ?? "", champion: bans.against.short })
      : m.plugin_lol_cs_bans_hint({ lane: bans.lane ?? "" })}</p>
    {#each bans.list as b, i (i)}
      <div class="line" data-testid="lol-cs-ban" data-reason={b.reason}>
        <ChampIcon id={b.champion.id} icon={b.champion.icon} name={b.champion.short} size={20} />
        <span class="name">{b.champion.short}</span>
        <span class="fact num" class:bad={b.reason === "counter"}>{b.reason === "counter"
          ? m.plugin_lol_cs_bans_counter({ rate: rate(b.winRate) })
          : m.plugin_lol_cs_bans_strong({ tier: b.tier, rate: rate(b.winRate) })}</span>
      </div>
    {/each}
  </section>
{/if}

{#if pool}
  <section class="blk" data-testid="lol-cs-pool">
    <header class="bh">
      <span class="bt">{m.plugin_lol_cs_pool_title()}</span>
      <SourceTag id={pool.source} {onLink} />
    </header>
    <p class="hint">{m.plugin_lol_cs_pool_hint()}{#if pool.lane}{` · ${pool.lane}`}{/if}</p>
    {#each pool.list as p, i (i)}
      <div class="pool" data-testid="lol-cs-pool-row" data-champion={p.champion.id}>
        <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={24} />
        <div class="pool-main">
          <span class="pool-top">
            <span class="name">{p.champion.short}</span>
            <span class="sub num">{m.plugin_lol_cs_pool_games({ games: p.games, wins: p.wins })}</span>
            {#if p.tier}<span class="sub num">{`${p.tier} · ${rate(p.winRate)}`}</span>{/if}
          </span>
          <span class="pool-vs">
            {#if p.good.length || p.bad.length}
              {#if p.good.length}<span class="good">{m.plugin_lol_good_against()}</span>
                {#each p.good as v, j (j)}<span class="vs num">{v.champion.short}{v.winRate !== null ? ` ${rate(v.winRate)}` : ""}</span>{/each}{/if}
              {#if p.bad.length}<span class="bad">{m.plugin_lol_bad_against()}</span>
                {#each p.bad as v, j (j)}<span class="vs num">{v.champion.short}{v.winRate !== null ? ` ${rate(v.winRate)}` : ""}</span>{/each}{/if}
            {:else}
              <span class="sub">{m.plugin_lol_cs_pool_none()}</span>
            {/if}
          </span>
        </div>
      </div>
    {/each}
  </section>
{/if}

{#if matchup}
  <section class="blk" data-testid="lol-cs-matchup">
    <header class="bh">
      <span class="bt">{m.plugin_lol_cs_matchup_title()}</span>
      <SourceTag id={matchup.source} {onLink} />
    </header>
    <div class="line vs-line">
      <ChampIcon id={matchup.me.id} icon={matchup.me.icon} name={matchup.me.short} size={20} />
      <span class="name">{m.plugin_lol_cs_matchup_vs({ me: matchup.me.short, opponent: matchup.opponent.short })}{#if matchup.lane}<span class="sub">{` · ${matchup.lane}`}</span>{/if}</span>
      <ChampIcon id={matchup.opponent.id} icon={matchup.opponent.icon} name={matchup.opponent.short} size={20} />
    </div>
    {#if matchup.guessed}<p class="hint">{m.plugin_lol_cs_matchup_guessed()}</p>{/if}
    <dl class="facts">
      {#if matchup.laneAdvantage}<div><dt>{m.plugin_lol_cs_matchup_lane()}</dt><dd data-testid="lol-cs-matchup-lane">{side(matchup.laneAdvantage, matchup)}</dd></div>{/if}
      {#if matchup.soloKillAdvantage}<div><dt>{m.plugin_lol_cs_matchup_solo()}</dt><dd>{side(matchup.soloKillAdvantage, matchup)}</dd></div>{/if}
      {#if matchup.playStyle}<div><dt>{m.plugin_lol_cs_matchup_style()}</dt><dd>{style(matchup.playStyle)}</dd></div>{/if}
    </dl>
    {#if matchup.tip}
      <details class="tip">
        <summary>{m.plugin_lol_cs_matchup_tip()}</summary>
        <p lang="en">{matchup.tip}</p>
      </details>
    {/if}
  </section>
{/if}

{#if comp && (comp.ally || comp.enemy)}
  <section class="blk" data-testid="lol-cs-comp">
    <header class="bh">
      <span class="bt">{m.plugin_lol_cs_comp_title()}</span>
      <SourceTag id={comp.source} {onLink} />
    </header>
    {#if comp.ally}{@render sideRow(m.plugin_lol_cs_comp_ally({ count: comp.ally.count }), comp.ally, "lol-cs-comp-ally")}{/if}
    {#if comp.enemy}{@render sideRow(m.plugin_lol_cs_comp_enemy({ count: comp.enemy.count }), comp.enemy, "lol-cs-comp-enemy")}{/if}
    <p class="hint">{m.plugin_lol_cs_comp_rule()}</p>
  </section>
{/if}

<style>
  .blk {
    margin: 6px 0 0;
    padding: 6px 12px 4px;
    border-top: 1px solid var(--lol-line);
  }
  .bh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 22px;
  }
  .bt {
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-strong);
    white-space: nowrap;
  }
  .hint {
    margin: 0 0 4px;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
    color: var(--lol-text-3);
  }
  .line {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    min-height: 26px;
    white-space: nowrap;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-medium);
  }
  .sub {
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-2);
  }
  .fact {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .fact.bad,
  .bad {
    color: var(--lol-loss);
  }
  .good {
    color: var(--lol-win);
  }
  .vs-line {
    grid-template-columns: 20px minmax(0, 1fr) 20px;
  }
  .pool {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr);
    align-items: start;
    column-gap: 8px;
    padding: 4px 0;
  }
  .pool + .pool {
    border-top: 1px dashed var(--lol-line);
  }
  .pool-main {
    display: grid;
    gap: 1px;
    min-width: 0;
  }
  .pool-top {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    white-space: nowrap;
  }
  .pool-vs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 6px;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
  }
  .vs {
    color: var(--lol-text-2);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 14px;
    margin: 2px 0 4px;
    font-size: var(--lol-fs-xs);
  }
  .facts div {
    display: flex;
    gap: 4px;
  }
  .facts dt {
    color: var(--lol-text-3);
  }
  .facts dd {
    margin: 0;
    font-weight: var(--lol-fw-medium);
  }
  .tip {
    margin: 0 0 4px;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
    color: var(--lol-text-2);
  }
  .tip summary {
    cursor: pointer;
    color: var(--lol-text-3);
  }
  .tip p {
    margin: 2px 0 0;
    overflow-wrap: anywhere;
  }
  .comp-side {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 10px;
    padding: 2px 0;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
  }
  .comp-side .k {
    font-weight: var(--lol-fw-medium);
  }
  .comp-side .v {
    color: var(--lol-text-2);
  }
  .flags {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
  }
</style>
