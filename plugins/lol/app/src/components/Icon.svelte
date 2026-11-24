<script lang="ts" module>
  // 线条图标（自己画的，和样张一样：24 网格、1.8 线宽、圆头）。不引图标库，单文件 App 越小越好。
  export type IconName = "refresh" | "target" | "eyeOff" | "open" | "info" | "back" | "map" | "search" | "alert";
</script>

<script lang="ts">
  let { name, spin = false }: { name: IconName; spin?: boolean } = $props();
</script>

<svg class="ico" class:spin viewBox="0 0 24 24" aria-hidden="true">
  {#if name === "refresh"}
    <path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" />
  {:else if name === "target"}
    <circle cx="12" cy="12" r="7" /><path d="M12 2v5M12 17v5M2 12h5M17 12h5" />
  {:else if name === "eyeOff"}
    <path d="M3 3l18 18" /><path
      d="M10.6 5.1A9.7 9.7 0 0 1 12 5c5 0 8.5 4.5 9.5 7a13 13 0 0 1-2.6 3.7M6.4 6.4A13 13 0 0 0 2.5 12c1 2.5 4.5 7 9.5 7a9.6 9.6 0 0 0 5.6-1.8"
    /><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
  {:else if name === "open"}
    <path d="M14 4h6v6M20 4l-8 8" /><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" />
  {:else if name === "info"}
    <circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7.5v.01" />
  {:else if name === "back"}
    <path d="M15 5l-7 7 7 7" />
  {:else if name === "search"}
    <circle cx="11" cy="11" r="6.5" /><path d="M16 16l4.5 4.5" />
  {:else if name === "alert"}
    <path d="M12 4 2.8 19.5h18.4L12 4Z" /><path d="M12 10v4.5M12 17v.01" />
  {:else if name === "map"}
    <!-- 小地图（插件 logo 的线稿）：方框是地图，对角线是中路，左下实心是我方基地 -->
    <rect x="4" y="4" width="16" height="16" rx="3" />
    <path d="M9.5 14.5 14.5 9.5" /><path d="M6.8 7.6c2 .8 2.4 2.4 4.4 3.6s2.4 2.8 4.4 3.6" opacity=".45" />
    <path d="M4 15a5 5 0 0 1 5 5H7a3 3 0 0 1-3-3Z" fill="currentColor" /><path d="M15 4a5 5 0 0 0 5 5" />
  {/if}
</svg>
