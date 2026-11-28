<script lang="ts">
  // 今天的状态（`ui://kotone/lol/today`，ADR 043 B 轮第 3 条；讨论记录第二节“排队 / 房间”：“今天的状态（几连胜 / 连败、
  // 哪个英雄一直输）”）。只在推送里出现：进房间或排队时插件推来，一天一张，打完一局回到房间时同一张原地更新
  // （宿主再推一次结果，这里跟着重画）。只写事实，都从我自己的战绩算：今天几局几胜几负、胜率、现在几连胜 / 连败、
  // 今天每一局的英雄小头像（新的在前，和“这局”的近况一样）、今天输了 2 局以上的英雄、各个模式几局。
  // 要窄的时候也看得清（悬浮窗的小窗约 360 px）：一行一件事，小头像放不下就换行。
  // 头部下面一行是插件挑的一句预置的话（`cheer`，ADR 050 第 5 条：连胜连败、胜负差得开时才有）。
  import { onMount } from "svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import Recent from "../components/Recent.svelte";
  import type { PageId } from "../components/TopBar.svelte";
  import type { DisplayMode, Session, ToolResult } from "../host";
  import { cheerLine } from "../format";
  import { m } from "../i18n.svelte";
  import { isTodayData, type TodayData } from "../types";

  let {
    session,
    initial,
    display,
    onTab,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    onTab: (page: PageId) => void;
  } = $props();

  let today = $state<TodayData | null>(null);
  let loaded = $state(false);
  const full = $derived(display !== "inline");
  const canOpen = $derived(!!session.context?.availableDisplayModes?.includes("fullscreen"));

  function show(result: ToolResult | null) {
    loaded = true;
    if (result && isTodayData(result.structuredContent)) today = result.structuredContent;
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then(show);
    else loaded = true;
  });

  /** `2026-11-26` → “11 月 26 日”。 */
  const dateLabel = (date: string) => {
    const [, month, day] = date.split("-").map(Number);
    return month && day ? m.plugin_lol_today_date({ month, day }) : date;
  };
  const rate = $derived(today && today.games > 0 ? Math.round((today.wins / today.games) * 100) : 0);
  const streakText = $derived(
    !today || today.streak === 0 ? ""
    : today.streak >= 2 ? m.plugin_lol_today_win_streak({ count: today.streak })
    : today.streak <= -2 ? m.plugin_lol_today_loss_streak({ count: -today.streak })
    : today.streak > 0 ? m.plugin_lol_today_last_win() : m.plugin_lol_today_last_loss(),
  );
  const cheer = $derived(cheerLine(today?.cheer));
  const streakTone = $derived(!today || Math.abs(today.streak) < 2 ? "" : today.streak > 0 ? "good" : "bad");
</script>

<div class="app" data-testid="lol-app-today" data-date={today?.date ?? ""}>
  {#if today}
    <div class="day" class:full>
      <div class="card-head">
        <span class="t">{m.plugin_lol_page_today()}</span>
        <span class="sub num">{dateLabel(today.date)}</span>
        {#if !full && canOpen}
          <button type="button" class="btn icon" title={m.plugin_lol_open_full()} aria-label={m.plugin_lol_open_full()} data-testid="lol-card-open"
            onclick={() => void session.requestDisplayMode("fullscreen")}><Icon name="open" /></button>
        {/if}
      </div>
      {#if cheer}<p class="card-cheer" data-testid="lol-today-cheer">{cheer}</p>{/if}
      <div class="line head" data-testid="lol-today-head">
        <b class="num">{m.plugin_lol_today_head({ games: today.games, wins: today.wins, losses: today.losses })}</b>
        <span class="muted num">{m.plugin_lol_today_rate({ rate })}</span>
        {#if streakText}
          <span class="chip {streakTone}" class:plain={!streakTone} data-testid="lol-today-streak" data-streak={today.streak}>{streakText}</span>
        {/if}
      </div>
      <div class="line recent" data-testid="lol-today-recent"><Recent games={today.recent} size={22} /></div>
      {#if today.lostOften.length}
        <p class="sub-title">{m.plugin_lol_today_lost_often({ count: today.lostAtLeast })}</p>
        <div class="lost">
          {#each today.lostOften as c (c.champion.id)}
            <div class="lrow stripe" data-testid="lol-today-lost" data-champion={c.champion.id}>
              <ChampIcon id={c.champion.id} icon={c.champion.icon} name={c.champion.short} size={22} />
              <span class="name">{c.champion.short}</span>
              <span class="num rec">{m.plugin_lol_today_record({ games: c.games, wins: c.wins, losses: c.losses })}</span>
            </div>
          {/each}
        </div>
      {/if}
      {#if today.modes.length > 1}
        <p class="line modes muted" data-testid="lol-today-modes">
          {today.modes.map((x) => m.plugin_lol_today_mode({ queue: x.queue, games: x.games, wins: x.wins })).join(" · ")}
        </p>
      {/if}
      {#if full}
        <div class="line"><button type="button" class="btn" data-testid="lol-today-open-player" onclick={() => onTab("player")}>{m.plugin_lol_today_open_player()}</button></div>
      {/if}
      <Footer card sources={[{ id: "client" }]} extra={today.capped ? m.plugin_lol_today_capped({ count: today.limit }) : ""} onLink={session.openLink} />
    </div>
  {:else if loaded}
    {#if full}
      <div class="empty-state" data-testid="lol-app-status"><p>{m.plugin_lol_today_empty()}</p></div>
    {:else}
      <p class="card-line" data-testid="lol-app-status">{m.plugin_lol_today_empty()}</p>
    {/if}
  {/if}
</div>

<style>
  .day.full {
    max-width: 560px;
    padding: var(--lol-sp-4) 0;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    margin: 0;
    padding: 2px 12px 4px;
  }
  .head b {
    font-size: var(--lol-fs-md);
    font-weight: var(--lol-fw-strong);
  }
  .muted {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .chip.plain {
    background: var(--lol-bg-3);
  }
  .recent :global(.recent) {
    flex-wrap: wrap;
  }
  .sub-title {
    margin: 6px 0 2px;
    padding: 0 12px;
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-medium);
    color: var(--lol-text-2);
  }
  .lost {
    --team: var(--lol-loss);
  }
  .lrow {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    height: 30px;
    padding: 0 12px 0 14px;
  }
  .lrow + .lrow {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--lol-fs-sm);
  }
  .rec {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
    white-space: nowrap;
  }
  .modes {
    padding-top: 6px;
  }
</style>
