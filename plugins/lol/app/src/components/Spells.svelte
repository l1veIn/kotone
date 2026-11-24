<script lang="ts">
  // 两个召唤师技能，竖排在英雄头像右边（19 px 两个正好 40 高）。没有数据时两个灰格子。
  import type { Iconed } from "../types";
  import Pic from "./Pic.svelte";

  let { spells = [], size = 19 }: { spells?: Iconed[]; size?: number } = $props();
  const two = $derived([spells[0] ?? null, spells[1] ?? null]);
</script>

<span class="spells">
  {#each two as spell, i (i)}
    {#if spell}
      <Pic path={spell.icon} name={spell.name || String(spell.id)} {size} />
    {:else}
      <span class="ci s{size}" aria-hidden="true"></span>
    {/if}
  {/each}
</span>

<style>
  .spells {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: 2px;
  }
</style>
