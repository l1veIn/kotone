<script lang="ts">
  // 覆盖层：选英雄时贴在客户端右上角的一小块（照 aramhex）。己方 5 个英雄的胜率（海斗：英雄榜名次；峡谷：那一路的强度），
  // 大乱斗 / 海斗还有备选席。只读、不能点、没有动画；字号跟着宿主给的区域大小走（cqh），放不下就截掉。
  import type { PluginOverlayProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import type { ChampSelectOverlayData } from "./types";
  import { rate } from "./format";

  let { data }: PluginOverlayProps = $props();
  const panel = $derived(data as ChampSelectOverlayData | null);
  const extra = (row: { rank?: number | null; tier?: string | null; lane?: string | null }) =>
    row.tier ? `${row.lane ?? ""} ${row.tier}`.trim() : row.rank ? m.plugin_lol_overlay_rank({ rank: row.rank }) : "";
</script>

{#if panel}
  <div class="panel" data-testid="lol-overlay-champ-select" data-mode={panel.mode}>
    <p class="title">{panel.mode === "mayhem" ? m.plugin_lol_overlay_team_mayhem() : m.plugin_lol_overlay_team_rift()}</p>
    {#each panel.team as row, i (i)}
      <p class="row" class:me={row.me} data-testid="lol-overlay-team-row">
        <span class="name">{row.champion}{#if row.me}<span class="me-tag">{m.plugin_lol_me()}</span>{/if}</span>
        <span class="rate">{rate(row.winRate)}</span>
        <span class="extra">{extra(row)}</span>
      </p>
    {/each}
    {#if panel.bench.length}
      <p class="bench" data-testid="lol-overlay-bench">
        <span class="label">{m.plugin_lol_overlay_bench()}</span>
        {#each panel.bench as row, i (i)}<span class="chip">{row.champion} {rate(row.winRate)}</span>{/each}
      </p>
    {/if}
    <p class="source">{panel.source}</p>
  </div>
{/if}

<style>
  .panel {
    box-sizing: border-box; width: 100%; height: 100%; overflow: hidden; padding: 4cqh 5cqw;
    display: flex; flex-direction: column; gap: 1.2cqh;
    background: rgb(10 12 18 / 0.84); border: 1px solid rgb(255 255 255 / 0.14); border-radius: 6px;
    color: #eef1f6; font-size: clamp(9px, 7.2cqh, 15px); line-height: 1.25; font-family: inherit;
    text-shadow: 0 1px 1px #000;
  }
  p { margin: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title { color: #9fb3c8; font-size: 0.85em; }
  .row { display: flex; gap: 0.6em; align-items: baseline; }
  .row.me { color: #7dd3fc; font-weight: 600; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .me-tag { margin-left: 0.3em; padding: 0 0.25em; border-radius: 3px; background: rgb(125 211 252 / 0.2); font-size: 0.8em; }
  .rate { font-variant-numeric: tabular-nums; }
  .extra { color: #9fb3c8; font-size: 0.85em; min-width: 2.6em; text-align: right; }
  .bench { display: flex; gap: 0.4em; align-items: baseline; color: #cfd8e3; font-size: 0.9em; }
  .bench .label { color: #9fb3c8; }
  .chip { padding: 0 0.3em; border-radius: 3px; background: rgb(255 255 255 / 0.08); }
  .source { margin-top: auto; color: #7b8794; font-size: 0.75em; }
</style>
