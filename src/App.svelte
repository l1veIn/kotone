<script lang="ts">
  import { onMount } from "svelte";
  import Overlay from "./routes/overlay/Overlay.svelte";
  import Settings from "./routes/settings/Settings.svelte";
  import { initStateListeners } from "./lib/stores/state";

  // 订阅 Rust 侧 kotone:// 事件（浏览器环境下为 no-op）
  onMount(() => {
    let cleanup: (() => void) | undefined;
    void initStateListeners().then((u) => (cleanup = u));
    return () => cleanup?.();
  });

  /*
   * 单 SPA 多窗口路由方案：
   * Tauri 两个窗口分别加载 index.html#/overlay 与 index.html#/settings，
   * 前端按 location.hash 决定渲染哪个视图，并监听 hashchange 便于浏览器调试。
   */
  let hash = $state(window.location.hash);

  $effect(() => {
    const onHashChange = () => {
      hash = window.location.hash;
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  });

  const view = $derived(hash.replace(/^#\/?/, "").split(/[/?]/)[0]);
</script>

{#if view === "overlay"}
  <Overlay />
{:else}
  <!-- 默认渲染设置视图（含 #/settings 与空 hash，便于纯浏览器开发预览） -->
  <Settings />
{/if}
