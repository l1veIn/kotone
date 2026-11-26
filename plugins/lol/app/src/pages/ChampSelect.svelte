<script lang="ts" module>
  /** 评级（照 AramHex：按国服胜率排名，前 10% S、30% A、60% B、85% C、其余 D；plugins/lol/src/pushes/rules.rs）。 */
  export type Grade = "S" | "A" | "B" | "C" | "D";
  export interface SelectChampion { id: number; short: string; icon?: string }
  /** 选英雄卡片的数据（推送 `champ-select`，后端 plugins/lol/src/pushes/champ_select.rs）。海斗：`rank` 是只按胜率排的名次
   * （共 `total` 个），`grade` 由它换算；备选席按胜率从高到低，`better` 是胜率比我现在的英雄高，`best` 是其中最高的。
   * 峡谷：`lane` + `tier`（腾讯 101 的强度）。`ended`：选英雄结束了（最后推的那一次）。 */
  export interface ChampSelectData {
    mode: "mayhem" | "rift";
    total?: number;
    team: { champion: SelectChampion; me: boolean; winRate: number | null; rank?: number | null; grade?: Grade | null; lane?: string | null; tier?: string | null }[];
    bench: { champion: SelectChampion; winRate: number | null; rank?: number | null; grade?: Grade | null; better?: boolean; best?: boolean }[];
    /** 数据从哪来：现在两种都是腾讯 101 的榜。 */
    source: string;
    ended?: boolean;
  }
  export function isChampSelect(value: unknown): value is ChampSelectData {
    return !!value && typeof value === "object" && Array.isArray((value as ChampSelectData).team) && Array.isArray((value as ChampSelectData).bench);
  }
</script>

<script lang="ts">
  // 选英雄（`ui://kotone/lol/champ-select`，ADR 043 D7：原来贴在客户端右上角的选英雄覆盖层，改成推送的一张卡片）。
  // 只在推送里出现：选英雄时插件推来，重随、换英雄时同一张原地更新（宿主再推一次结果，这里跟着重画）。
  // 海斗照 AramHex 的选英雄标注：每个人一个评级小牌（S–D）、胜率、第几 / 共几；备选席按胜率从高到低，比你现在的英雄高的
  // 标 ↑，最高的那个用墨线圈出来（值得换）。峡谷：每个人这个位置的强度和胜率。
  // 要窄的时候也看得清（悬浮窗的小窗约 360 px）：一人一行，列固定宽；备选宽的时候两个一行，窄的时候一个一行。
  // 只学 AramHex 标什么，视觉是“记分纸”（docs/design/lol-app.md），不抄代码。
  import { onMount } from "svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import type { DisplayMode, Session, ToolResult } from "../host";
  import { m } from "../i18n.svelte";

  let {
    session,
    initial,
    display,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
  } = $props();

  let panel = $state<ChampSelectData | null>(null);
  let loaded = $state(false);
  const full = $derived(display !== "inline");
  const mayhem = $derived(panel?.mode === "mayhem");
  const anyBetter = $derived(panel?.bench.some((b) => b.better) ?? false);

  function show(result: ToolResult | null) {
    loaded = true;
    if (result && isChampSelect(result.structuredContent)) panel = result.structuredContent;
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then(show);
    else loaded = true;
  });

  const rate = (value: number | null | undefined) =>
    typeof value === "number" && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—";
  const extra = (row: ChampSelectData["team"][number]) =>
    row.tier ? [row.lane, row.tier].filter(Boolean).join(" ")
    : row.rank && panel?.total ? m.plugin_lol_champ_select_rank({ rank: row.rank, total: panel.total }) : "";
</script>

<div class="app" data-testid="lol-app-champ-select" data-mode={panel?.mode ?? ""} data-ended={panel?.ended ? "true" : "false"}>
  {#if panel}
    <div class="sel" class:full>
      <div class="card-head">
        <span class="t">{m.plugin_lol_page_champ_select()}</span>
        <span class="sub">{panel.ended ? m.plugin_lol_champ_select_ended() : mayhem ? m.plugin_lol_champ_select_team_mayhem() : m.plugin_lol_champ_select_team_rift()}</span>
      </div>
      <div class="rows" data-testid="lol-champ-select-team">
        {#each panel.team as row, i (i)}
          <div class="row stripe ally" class:me={row.me} data-testid="lol-champ-select-row" data-grade={row.grade ?? ""} data-me={row.me ? "true" : "false"}>
            <ChampIcon id={row.champion.id} icon={row.champion.icon} name={row.champion.short} size={24} />
            <span class="name">{row.champion.short}{#if row.me}<span class="chip me">{m.plugin_lol_me()}</span>{/if}</span>
            {#if mayhem}<span class="grade" data-grade={row.grade ?? ""}>{row.grade ?? "–"}</span>{/if}
            <span class="rate num">{rate(row.winRate)}</span>
            <span class="extra num">{extra(row)}</span>
          </div>
        {/each}
      </div>
      {#if panel.bench.length}
        <p class="bench-title">{m.plugin_lol_champ_select_bench()}{#if anyBetter}<span class="hint">{` · ${m.plugin_lol_champ_select_bench_better()}`}</span>{/if}</p>
        <div class="bench" data-testid="lol-champ-select-bench">
          {#each panel.bench as seat, i (i)}
            <div class="seat" class:better={seat.better} class:best={seat.best} data-testid="lol-champ-select-seat"
              data-better={seat.better ? "true" : "false"} data-best={seat.best ? "true" : "false"} title={seat.best ? m.plugin_lol_champ_select_best() : undefined}>
              <ChampIcon id={seat.champion.id} icon={seat.champion.icon} name={seat.champion.short} size={20} />
              <span class="name">{seat.champion.short}</span>
              <span class="grade" data-grade={seat.grade ?? ""}>{seat.grade ?? "–"}</span>
              <span class="rate num">{rate(seat.winRate)}{#if seat.better}<span class="up" aria-hidden="true">↑</span>{/if}</span>
              {#if seat.best}<span class="sr-only">{m.plugin_lol_champ_select_best()}</span>{/if}
            </div>
          {/each}
        </div>
      {/if}
      <div class="card-foot">{m.plugin_lol_champ_select_source({ source: panel.source })}</div>
    </div>
  {:else if loaded}
    {#if full}
      <div class="empty-state" data-testid="lol-app-status"><p>{m.plugin_lol_champ_select_empty()}</p></div>
    {:else}
      <p class="card-line" data-testid="lol-app-status">{m.plugin_lol_champ_select_empty()}</p>
    {/if}
  {/if}
</div>

<style>
  .sel.full {
    max-width: 560px;
    padding: var(--lol-sp-4) 0;
  }
  .rows {
    margin: 0 0 2px;
  }
  .row {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) auto 4.2em 5.6em;
    align-items: center;
    column-gap: 8px;
    height: var(--lol-row-card);
    padding: 0 12px 0 14px;
    white-space: nowrap;
  }
  .row + .row {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .row.me {
    background: var(--lol-ally-soft);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--lol-fw-strong);
  }
  .rate {
    text-align: right;
  }
  .extra {
    text-align: right;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  /* 评级字母小牌（S–D）：S 胜色、A 我方蓝、B 正文色、C 提醒色、D 负色；字母本身就能分，颜色只是帮着看 */
  .grade {
    --g: var(--lol-text);
    display: inline-grid;
    place-items: center;
    min-width: 20px;
    height: 18px;
    padding: 0 3px;
    border-radius: var(--lol-r-xs);
    font-size: var(--lol-fs-xs);
    font-weight: 700;
    line-height: 1;
    color: var(--g);
    background: color-mix(in srgb, var(--g) 12%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--g) 45%, transparent);
  }
  .grade[data-grade="S"] { --g: var(--lol-win); }
  .grade[data-grade="A"] { --g: var(--lol-ally); }
  .grade[data-grade="C"] { --g: var(--lol-warn); }
  .grade[data-grade="D"] { --g: var(--lol-loss); }
  .grade[data-grade=""] { --g: var(--lol-text-3); }
  .bench-title {
    margin: 8px 14px 4px;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
  }
  .hint {
    color: var(--lol-text-2);
  }
  .bench {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 4px 6px;
    padding: 0 12px;
  }
  .seat {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto auto;
    align-items: center;
    column-gap: 6px;
    min-width: 0;
    height: 28px;
    padding: 0 6px 0 4px;
    border-radius: var(--lol-r-md);
    background: var(--lol-bg-2);
    color: var(--lol-text-2);
    white-space: nowrap;
  }
  .seat .name {
    font-weight: var(--lol-fw-normal);
  }
  .seat.better {
    color: var(--lol-win);
  }
  .seat.better .name {
    color: var(--lol-text);
  }
  /* 值得换的那个：墨线圈出来（不用金色，设计文档第四节 4） */
  .seat.best {
    box-shadow: inset 0 0 0 1.5px var(--lol-text);
  }
  .up {
    margin-left: 2px;
    font-weight: 700;
  }
  .card-foot {
    padding-top: 8px;
  }
  @container (max-width: 400px) {
    .row {
      grid-template-columns: 24px minmax(0, 1fr) auto 4.2em 4.6em;
      column-gap: 6px;
    }
    .bench {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
