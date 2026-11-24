<script lang="ts">
  // 装备：6 格 + 饰品（饰品和前 6 格隔 4 px），空格子是灰格子；名字在 title。
  import type { Item } from "../types";
  import Pic from "./Pic.svelte";

  let { items = [], size = 22, trinket = true }: { items?: Item[]; size?: number; trinket?: boolean } = $props();
  const slots = $derived(Array.from({ length: 6 }, (_, i) => items.find((x) => x.slot === i) ?? null));
  const ward = $derived(items.find((x) => x.slot === 6) ?? null);
</script>

<span class="items">
  {#each slots as item, i (i)}
    {#if item}<Pic path={item.icon} name={item.name || String(item.id)} {size} />{:else}<span class="ci s{size}" aria-hidden="true"></span>{/if}
  {/each}
  {#if trinket}
    <span class="gap"></span>
    {#if ward}<Pic path={ward.icon} name={ward.name || String(ward.id)} {size} />{:else}<span class="ci s{size}" aria-hidden="true"></span>{/if}
  {/if}
</span>

<style>
  .items {
    display: inline-flex;
    flex: none;
    gap: 2px;
  }
  .gap {
    width: 2px;
  }
</style>
