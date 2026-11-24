<script lang="ts" module>
  /** 分路（数据里的 `TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`，强度榜的 `SUPPORT`）→ 统一的写法；不认识的给空。 */
  export function laneOf(raw: string | null | undefined): "TOP" | "JUNGLE" | "MIDDLE" | "BOTTOM" | "UTILITY" | "" {
    const v = String(raw ?? "").toUpperCase();
    if (v === "TOP" || v === "JUNGLE" || v === "MIDDLE" || v === "BOTTOM" || v === "UTILITY") return v;
    if (v === "SUPPORT") return "UTILITY";
    if (v === "MID") return "MIDDLE";
    if (v === "BOT" || v === "ADC") return "BOTTOM";
    return "";
  }
</script>

<script lang="ts">
  // 分路图标（docs/design/lol-app.md 第一节 7，原创）：和插件 logo 是同一张小地图，淡淡的方框是地图，把那一条路描粗。
  let { lane, size = 16 }: { lane: string; size?: number } = $props();
  const l = $derived(laneOf(lane));
</script>

{#if l}
  <svg class="lane" width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-linecap="round" aria-hidden="true">
    <rect x="2.5" y="2.5" width="11" height="11" rx="2" stroke-opacity=".32" stroke-width="1.4" />
    {#if l === "TOP"}
      <path d="M3 11.5V5a2 2 0 0 1 2-2h6.5" stroke-width="2.2" />
    {:else if l === "MIDDLE"}
      <path d="M4.6 11.4 11.4 4.6" stroke-width="2.2" />
    {:else if l === "BOTTOM"}
      <path d="M4.5 13H11a2 2 0 0 0 2-2V4.5" stroke-width="2.2" />
    {:else if l === "JUNGLE"}
      <g fill="currentColor" stroke="none"><circle cx="5.2" cy="8" r="1.3" /><circle cx="8" cy="5.2" r="1.3" /><circle cx="10.8" cy="8" r="1.3" /><circle cx="8" cy="10.8" r="1.3" /></g>
    {:else}
      <path d="M4.5 13H11a2 2 0 0 0 2-2V4.5" stroke-width="1.4" stroke-opacity=".6" /><path d="M8 5.2v5.6M5.2 8h5.6" stroke-width="2" />
    {/if}
  </svg>
{/if}

<style>
  .lane {
    flex: none;
  }
</style>
