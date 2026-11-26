<script lang="ts">
  // 一队每个人的海克斯（ADR 043 B 轮第 3 条，讨论记录第二节“结算”那一行：“这局总结；再加一行对面每个人的海克斯”）：
  // 一人一行，左边英雄头像，右边按选的顺序一格一个海克斯（图标 + 名字，名字太长省略，完整的在 title）。
  // 格子按宽度排：悬浮窗的小窗（约 360 px）一行两格，聊天里（最宽 544 px）和全屏一行四格。
  // 数据是单局详情里每个人的 `augments`（客户端的 playerAugment1–6）；没有海克斯的人不列。
  import Augment, { rarityOf } from "../../components/Augment.svelte";
  import ChampIcon from "../../components/ChampIcon.svelte";
  import type { GamePlayer } from "../../types";

  let { players, side }: { players: GamePlayer[]; side: "ally" | "enemy" | "neutral" } = $props();
  const rows = $derived(players.filter((p) => p.augments.length > 0));
</script>

<div class="aug-rows {side}" data-testid="lol-game-augments" data-side={side}>
  {#each rows as p, i (p.puuid || i)}
    <div class="ar stripe" class:me={p.me} data-testid="lol-game-augment-row" data-champion={p.champion.id}>
      <ChampIcon id={p.champion.id} icon={p.champion.icon} name={p.champion.short} size={22} />
      <span class="list">
        {#each p.augments as a, j (`${a.id}-${j}`)}
          <span class="chip-aug" data-testid="lol-game-augment" data-rarity={rarityOf(a.rarity)} title={a.name || String(a.id)}>
            <Augment augment={a} size={18} /><span class="an">{a.name || a.id}</span>
          </span>
        {/each}
      </span>
    </div>
  {/each}
</div>

<style>
  .aug-rows {
    container-type: inline-size;
  }
  .ar {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr);
    align-items: start;
    column-gap: 8px;
    padding: 4px 12px 4px 14px;
  }
  .ar + .ar {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .ar.me {
    background: var(--lol-ally-soft);
  }
  .list {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 2px 8px;
    min-width: 0;
  }
  @container (min-width: 460px) {
    .list {
      /* 全屏很宽时格子不拉开（每格最多 12em），名字挨着图标读 */
      grid-template-columns: repeat(4, minmax(0, 12em));
    }
  }
  .chip-aug {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    height: 20px;
  }
  .an {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
</style>
