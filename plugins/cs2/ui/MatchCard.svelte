<script lang="ts">
  // 一张 CS2 的图（`ui://kotone/cs2/match`）：她调 cs2.state 时聊天里的卡片、比赛结束的推送卡片、插件页上的“这一局”都是它。
  // 只显示事实：比分、回合、你在哪边、你的数据、每回合谁赢；结束时写赢 / 输 / 平。
  import type { PluginViewProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { mapLabel, modeLabel, scoreSides, type MatchView } from "./cs2";

  let { data }: PluginViewProps = $props();
  const view = $derived(data && typeof data === "object" && (data as MatchView).playing ? (data as MatchView) : null);
  const score = $derived(view ? scoreSides(view) : null);
  const phaseText = $derived.by(() => {
    switch (view?.phase) {
      case "warmup": return m.plugin_cs2_phase_warmup();
      case "intermission": return m.plugin_cs2_phase_intermission();
      case "gameover": return m.plugin_cs2_phase_gameover();
      case "live": return m.plugin_cs2_phase_live();
      default: return view?.over ? m.plugin_cs2_phase_gameover() : "";
    }
  });
  const resultText = $derived(view?.result === "win" ? m.plugin_cs2_result_win() : view?.result === "loss" ? m.plugin_cs2_result_loss()
    : view?.result === "tie" ? m.plugin_cs2_result_tie() : "");
  const reason = (r: string) => ({ elimination: m.plugin_cs2_reason_elimination(), bomb: m.plugin_cs2_reason_bomb(),
    defuse: m.plugin_cs2_reason_defuse(), time: m.plugin_cs2_reason_time() } as Record<string, string>)[r] ?? "";
</script>

<div class="min-w-0" data-testid="cs2-match-card">
  {#if view && score}
    <div class="flex items-baseline justify-between gap-2 text-xs text-muted-foreground">
      <span class="min-w-0 truncate" data-testid="cs2-match-map">{mapLabel(view.map, view.mapName)}{view.mode ? ` · ${modeLabel(view.mode, view.modeName)}` : ""}</span>
      <span class="shrink-0" data-testid="cs2-match-phase">{phaseText}{view.round && !view.over ? ` · ${m.plugin_cs2_round({ round: view.round })}` : ""}</span>
    </div>
    <div class="mt-1 flex items-center gap-3">
      <div class="flex items-baseline gap-2 text-[22px] font-semibold tabular-nums text-foreground" data-testid="cs2-match-score">
        <span class="text-xs font-normal text-muted-foreground">{score.mine ? m.plugin_cs2_us() : "CT"}</span>
        <span>{score.left}</span><span class="text-muted-foreground">:</span><span>{score.right}</span>
        <span class="text-xs font-normal text-muted-foreground">{score.mine ? m.plugin_cs2_them() : "T"}</span>
      </div>
      {#if resultText}
        <span class="rounded px-1.5 py-0.5 text-xs font-medium {view.result === 'win' ? 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300' : view.result === 'loss' ? 'bg-rose-500/15 text-rose-700 dark:text-rose-300' : 'bg-muted text-muted-foreground'}"
          data-testid="cs2-match-result" data-result={view.result}>{resultText}</span>
      {/if}
      {#if view.myTeam}<span class="ml-auto text-xs text-muted-foreground" data-testid="cs2-match-side">{m.plugin_cs2_side({ side: view.myTeam })}</span>{/if}
    </div>
    {#if view.stats}
      <dl class="m-0 mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
        <dt class="text-muted-foreground">{m.plugin_cs2_kda()}</dt>
        <dd class="m-0 tabular-nums text-foreground" data-testid="cs2-match-kda">{view.stats.kills} / {view.stats.deaths} / {view.stats.assists}</dd>
        <dt class="text-muted-foreground">{m.plugin_cs2_mvps()}</dt>
        <dd class="m-0 tabular-nums text-foreground" data-testid="cs2-match-mvps">{view.stats.mvps}</dd>
        {#if view.me && !view.over}
          <dt class="text-muted-foreground">{m.plugin_cs2_health()} / {m.plugin_cs2_armor()}</dt>
          <dd class="m-0 tabular-nums text-foreground">{view.me.health} / {view.me.armor}</dd>
          <dt class="text-muted-foreground">{m.plugin_cs2_money()}</dt>
          <dd class="m-0 tabular-nums text-foreground">${view.me.money}</dd>
        {/if}
      </dl>
    {/if}
    {#if view.rounds?.length}
      <ol class="m-0 mt-2 flex list-none flex-wrap gap-1 p-0" aria-label={m.plugin_cs2_rounds()} data-testid="cs2-match-rounds">
        {#each view.rounds as r (r.round)}
          <li class="flex h-5 min-w-5 items-center justify-center rounded px-1 text-[10px] font-medium tabular-nums
            {r.won === true ? 'bg-emerald-500/20 text-emerald-800 dark:text-emerald-200' : r.won === false ? 'bg-rose-500/20 text-rose-800 dark:text-rose-200' : r.winner === 'CT' ? 'bg-sky-500/20 text-sky-800 dark:text-sky-200' : 'bg-amber-500/20 text-amber-800 dark:text-amber-200'}"
            title={reason(r.reason) ? m.plugin_cs2_round_won({ round: r.round, winner: r.winner, reason: reason(r.reason) }) : m.plugin_cs2_round_won_plain({ round: r.round, winner: r.winner })}
            data-testid="cs2-round" data-won={r.won === undefined ? "" : String(r.won)}>{r.round}</li>
        {/each}
      </ol>
    {/if}
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_cs2_match_empty()}</p>
  {/if}
</div>
