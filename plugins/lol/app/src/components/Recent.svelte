<script lang="ts">
  // 最近几局的英雄小头像：赢的亮、输的淡（50% 透明 + 去一半颜色），底下一条 2 px 的胜 / 负色线；
  // title 写“胜 5/4/12”。颜色从不单独表达意思：亮 / 淡也是一层。
  import { championIconPath } from "../assets";
  import { m } from "../i18n.svelte";
  import type { RecentGame } from "../types";
  import Pic from "./Pic.svelte";

  let { games, size = 18 }: { games: RecentGame[]; size?: number } = $props();
</script>

<span class="recent" role="img" aria-label={m.plugin_lol_recent({ count: games.length })}>
  {#each games as g (g.gameId)}
    {@const result = g.win ? m.plugin_lol_win() : m.plugin_lol_loss()}
    <Pic
      path={g.championIcon || championIconPath(g.championId)}
      name={result}
      title="{result} {g.kills}/{g.deaths}/{g.assists}"
      {size}
      crop
      cls={g.win ? "rg" : "rg l"}
    />
  {/each}
</span>

<style>
  .recent {
    display: flex;
    flex: none;
    gap: 2px;
  }
  .recent :global(.rg)::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 1;
    height: 2px;
    background: var(--lol-win);
  }
  .recent :global(.rg.l)::before {
    background: var(--lol-loss);
  }
  .recent :global(.rg.l img) {
    opacity: 0.5;
    filter: grayscale(0.6);
  }
</style>
