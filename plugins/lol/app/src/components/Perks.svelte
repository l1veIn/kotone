<script lang="ts">
  // 符文：基石 + 副系，竖排；圆形图标放在灰色圆底上（符文图是透明的）。海斗、斗魂竞技场没有符文时不画。
  import type { Perks } from "../types";
  import Pic from "./Pic.svelte";

  let { perks, size = 19, sub = 15 }: { perks: Perks | null | undefined; size?: number; sub?: number } = $props();
</script>

{#if perks && (perks.keystone || perks.subStyle)}
  <span class="perks">
    {#if perks.keystone}<Pic path={perks.keystone.icon} name={perks.keystone.name || String(perks.keystone.id)} {size} round />{/if}
    {#if perks.subStyle}<Pic path={perks.subStyle.icon} name={perks.subStyle.name || String(perks.subStyle.id)} size={sub} round />{/if}
  </span>
{/if}

<style>
  .perks {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
</style>
