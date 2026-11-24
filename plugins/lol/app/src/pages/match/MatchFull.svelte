<script lang="ts">
  // “这局”全屏（插件页、Claude 的全屏），照 docs/design/lol-app.md 第三节 1 分四种状态：
  // 选人（我方每人一块、对面只有英雄、备选席）、加载 / 对局中（10 个人）、结算（顶上“刚结束”，下面照旧是 10 个人）。
  // 宽 ≥ 880 两队并排（我方左、对面右）；窄时上下叠，对面在上。
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Icon from "../../components/Icon.svelte";
  import { duration } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchData, MatchPlayer } from "../../types";
  import { partyOf, phaseOf, pickOf } from "./match";
  import PlayerBlock from "./PlayerBlock.svelte";
  import TeamHead from "./TeamHead.svelte";

  let {
    match,
    onPlayer,
    onGame,
    onMine,
  }: {
    match: MatchData;
    onPlayer: (p: MatchPlayer) => void;
    onGame: (gameId: number) => void;
    onMine: () => void;
  } = $props();

  const phase = $derived(phaseOf(match));
  const champSelect = $derived(phase === "champ_select");
  const party = $derived(partyOf(match));
  /** 结算后过了这么久还没有结果，就不再等（设计第三节 1“结算”）。 */
  const RESULT_GIVE_UP_MS = 60_000;
  const gaveUp = $derived(
    !!match.resultPending && typeof match.phaseSince === "number" && Date.now() - match.phaseSince > RESULT_GIVE_UP_MS,
  );
</script>

{#if phase === "post_match"}
  <section class="result" data-testid="lol-match-result" aria-label={m.plugin_lol_just_ended()}>
    <span class="result-tag">{m.plugin_lol_just_ended()}</span>
    {#if match.result}
      <span class="outcome {match.result.win ? 'win' : 'loss'}">{match.result.win ? m.plugin_lol_victory() : m.plugin_lol_defeat()}</span>
      <span class="result-sub num">{match.mode} · {duration(match.result.durationS)}</span>
      <button type="button" class="btn" data-testid="lol-match-result-open" onclick={() => match.result && onGame(match.result.gameId)}>
        {m.plugin_lol_open_game()}
      </button>
    {:else if gaveUp || !match.resultPending}
      <span class="result-sub">{m.plugin_lol_result_missing()}</span>
      <button type="button" class="btn" onclick={onMine}>{m.plugin_lol_see_mine()}</button>
    {:else}
      <span class="result-sub">{m.plugin_lol_result_pending()}</span>
    {/if}
  </section>
  <h2 class="section">{m.plugin_lol_start_records()}</h2>
{/if}

{#if champSelect && match.namesHidden}
  <p class="banner" data-testid="lol-match-names-hidden"><Icon name="info" />{m.plugin_lol_names_hidden_note()}</p>
{/if}

{#if !champSelect || match.waiting > 0}
  <p class="note">
    {#if !champSelect}<span>{m.plugin_lol_note_recent()}</span>{/if}
    {#if match.waiting > 0}<span class="warn">{m.plugin_lol_waiting_count({ count: match.waiting })}</span>{/if}
    {#if match.random && !champSelect}<span>{m.plugin_lol_random_short()}</span>{/if}
  </p>
{/if}

<div class="teams">
  <section class="team ally" aria-label={m.plugin_lol_allies()} data-testid="lol-match-allies">
    <TeamHead ally team={match.allies} {party} />
    {#each match.allies as p, i (p.puuid || `a${i}`)}
      <PlayerBlock {p} party={party(p)} mode={match.mode} {champSelect} onOpen={onPlayer} />
    {/each}
  </section>
  <section class="team enemy" aria-label={m.plugin_lol_enemies()} data-testid="lol-match-enemies">
    <TeamHead ally={false} team={match.enemies} {party} stats={!champSelect} />
    {#if champSelect}
      {#each match.enemies as p, i (p.puuid || `e${i}`)}
        {@const pick = pickOf(p)}
        <div class="pick stripe" data-testid="lol-match-enemy-pick" data-pick={pick}>
          {#if pick === "none"}
            <span class="ci s40 empty" aria-hidden="true"></span><span class="muted">{m.plugin_lol_not_picked()}</span>
          {:else}
            <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} cls={pick === "intent" ? "intent" : ""} />
            <span class="champ">{p.champion.short}</span>
            {#if pick === "intent"}<span class="muted">{m.plugin_lol_pick_intent()}</span>{/if}
          {/if}
        </div>
      {:else}
        <p class="pick muted">{m.plugin_lol_enemy_unknown()}</p>
      {/each}
    {:else}
      {#each match.enemies as p, i (p.puuid || `e${i}`)}
        <PlayerBlock {p} party={party(p)} mode={match.mode} onOpen={onPlayer} />
      {/each}
    {/if}
  </section>
</div>

{#if champSelect && match.bench && match.bench.length > 0}
  <div class="bench" data-testid="lol-match-bench">
    <span class="bench-label">{m.plugin_lol_bench()}</span>
    <span class="bench-list">
      {#each match.bench as c (c.id)}<ChampIcon id={c.id} icon={c.icon} name={c.short} size={32} />{/each}
    </span>
  </div>
{/if}

<style>
  .teams {
    display: grid;
    gap: var(--lol-sp-5);
    grid-template-columns: minmax(0, 1fr);
  }
  .teams .team.enemy {
    order: -1;
  }
  @container (min-width: 880px) {
    .teams {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      align-items: start;
    }
    .teams .team.enemy {
      order: 0;
    }
  }
  /* 一人一块之间的细线（PlayerBlock 一个组件只有一块，兄弟选择器写在这里） */
  .team :global(.p + .p) {
    border-top: 1px solid var(--lol-line);
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    margin: 0;
    padding: 4px 12px 4px 14px;
  }
  .pick + .pick {
    border-top: 1px solid var(--lol-line);
  }
  .pick :global(.intent) {
    opacity: 0.5;
    outline: 1px dashed var(--lol-text-2);
    outline-offset: -1px;
  }
  .champ {
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-strong);
  }
  .muted {
    font-size: var(--lol-fs-sm);
    color: var(--lol-text-3);
  }
  .bench {
    display: flex;
    align-items: center;
    gap: var(--lol-sp-5);
    margin-top: var(--lol-sp-5);
    padding: 8px 12px;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-lg);
    background: var(--lol-bg-2);
  }
  .bench-label {
    flex: none;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .bench-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .result {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--lol-sp-4) var(--lol-sp-5);
    margin-bottom: var(--lol-sp-5);
    padding: 12px 16px;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-lg);
    background: var(--lol-bg-2);
  }
  .result-tag {
    padding: 0 6px;
    border-radius: var(--lol-r-xs);
    background: var(--lol-ink-fill);
    color: var(--lol-ink-on);
    font-size: var(--lol-fs-xs);
    line-height: 18px;
    font-weight: var(--lol-fw-medium);
  }
  .outcome {
    font-size: var(--lol-fs-2xl);
    line-height: var(--lol-lh-2xl);
    font-weight: 700;
  }
  .outcome.win {
    color: var(--lol-win);
  }
  .outcome.loss {
    color: var(--lol-loss);
  }
  .result-sub {
    font-size: var(--lol-fs-md);
    color: var(--lol-text-2);
  }
  .result .btn {
    margin-left: auto;
  }
  .section {
    margin: 0 0 var(--lol-sp-3);
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text-3);
  }
</style>
