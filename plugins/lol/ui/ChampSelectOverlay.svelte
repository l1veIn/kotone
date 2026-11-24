<script lang="ts">
  // 覆盖层：选英雄时贴在客户端右上角的一小块（照 AramHex 的选英雄标注，MIT）。
  // 海斗：每个英雄一个评级小牌（S–D，按国服胜率排名换算）、胜率、第几 / 共几；备选席按胜率从高到低，两个一行，
  // 胜率比你现在的英雄高的标 ↑ 和胜色，最高的那个用 2 px 白色墨线圈出来（值得换；设计文档第四节 4：不用金色）。
  // 峡谷：那一路的强度和胜率。只读、不能点、没有动画；字号跟着宿主给的区域宽度走（cqw），高度由插件按行数给。
  import "./overlay.css";
  import type { PluginOverlayProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { ChampSelectOverlayData } from "./types";
  import { rate } from "./format";

  let { data }: PluginOverlayProps = $props();
  const panel = $derived(data as ChampSelectOverlayData | null);
  const mayhem = $derived(panel?.mode === "mayhem");
  const anyBetter = $derived(panel?.bench.some((b) => b.better) ?? false);
  const extra = (row: ChampSelectOverlayData["team"][number]) =>
    row.tier ? `${row.lane ?? ""} ${row.tier}`.trim()
    : row.rank && panel?.total ? m.plugin_lol_overlay_rank({ rank: row.rank, total: panel.total }) : "";
</script>

{#if panel}
  <div class="lol-overlay panel" data-testid="lol-overlay-champ-select" data-mode={panel.mode}>
    <p class="title">{mayhem ? m.plugin_lol_overlay_team_mayhem() : m.plugin_lol_overlay_team_rift()}</p>
    {#each panel.team as row, i (i)}
      <p class="row" class:me={row.me} data-testid="lol-overlay-team-row" data-grade={row.grade ?? ""}>
        {#if mayhem}<span class="lol-grade" data-grade={row.grade ?? ""}>{row.grade ?? "–"}</span>{/if}
        <span class="name">{row.champion}{#if row.me}<span class="me-tag">{m.plugin_lol_me()}</span>{/if}</span>
        <span class="rate">{rate(row.winRate)}</span>
        <span class="extra">{extra(row)}</span>
      </p>
    {/each}
    {#if panel.bench.length}
      <p class="title bench-title">{m.plugin_lol_overlay_bench()}{#if anyBetter}<span class="hint">{` · ${m.plugin_lol_overlay_bench_better()}`}</span>{/if}</p>
      <div class="bench" data-testid="lol-overlay-bench">
        {#each panel.bench as seat, i (i)}
          <span class="seat" class:better={seat.better} class:best={seat.best} data-testid="lol-overlay-bench-seat"
            data-grade={seat.grade ?? ""} data-better={seat.better ? "true" : "false"} data-best={seat.best ? "true" : "false"}>
            <span class="lol-grade" data-grade={seat.grade ?? ""}>{seat.grade ?? "–"}</span>
            <span class="name">{seat.champion}</span>
            <span class="rate">{rate(seat.winRate)}{#if seat.better}<span class="up">↑</span>{/if}</span>
          </span>
        {/each}
      </div>
    {/if}
    <p class="source">{panel.source}</p>
  </div>
{/if}

<style>
  .panel {
    padding: 3cqw 4.5cqw; display: flex; flex-direction: column; gap: 0.25em;
    font-size: clamp(9px, 5.2cqw, 22px); line-height: 1.3;
  }
  p { margin: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title { color: var(--ov-text-3); font-size: 0.85em; }
  .row { display: flex; gap: 0.45em; align-items: center; padding: 0 0.2em; border-radius: 3px; }
  .row.me { background: rgb(110 162 255 / 0.14); font-weight: 600; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .me-tag { margin-left: 0.35em; padding: 0 0.3em; border-radius: 3px; background: rgb(110 162 255 / 0.22); color: var(--ov-ally); font-size: 0.8em; }
  .rate { flex: none; }
  .extra { flex: none; min-width: 3.6em; color: var(--ov-text-2); font-size: 0.85em; text-align: right; }
  .bench-title { margin-top: 0.15em; }
  .hint { color: var(--ov-text-2); }
  .bench { display: grid; grid-template-columns: 1fr 1fr; gap: 0.2em 0.35em; font-size: 0.92em; }
  .seat {
    display: flex; align-items: center; gap: 0.3em; min-width: 0; padding: 0.1em 0.35em 0.1em 0.25em; border-radius: 4px;
    color: var(--ov-text-2); white-space: nowrap;
  }
  .seat.better { color: var(--ov-win); }
  .seat.better .name { color: var(--ov-text); }
  .seat.best { box-shadow: inset 0 0 0 2px var(--ov-text); }
  .up { margin-left: 0.1em; font-weight: 700; }
  .source { margin-top: auto; color: var(--ov-text-3); font-size: 0.75em; }
</style>
