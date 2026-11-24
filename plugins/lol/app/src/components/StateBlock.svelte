<script lang="ts">
  // 没数据、客户端没开、数据开关关着、查不到、出错（设计第一节 10）：全屏在页面中间（小地图线稿 + 一句话 + 下一步），
  // 卡片里一行字。文案按状态码选（status.ts），给琴音的原话放 title。
  import type { Snippet } from "svelte";
  import { m } from "../i18n.svelte";
  import { stateView, type ToolState } from "../status";
  import Icon from "./Icon.svelte";

  let {
    state,
    inline = false,
    onSettings,
    children,
  }: {
    state: ToolState;
    inline?: boolean;
    /** “去设置”（数据开关关着时）。 */
    onSettings?: () => void;
    /** 别的下一步（“看我的战绩”）。 */
    children?: Snippet;
  } = $props();

  const view = $derived(stateView(state));
</script>

{#if inline}
  <p class="card-line" class:danger={state.status === "error"} title={state.text} data-testid="lol-app-status" data-status={state.status}>{view.text}</p>
{:else}
  <div class="empty-state" data-testid="lol-app-status" data-status={state.status} data-reason={state.reason ?? ""}>
    <Icon name={state.status === "error" ? "alert" : "map"} />
    <p title={state.text}>{view.text}</p>
    <div class="actions">
      {#if view.settings && onSettings}
        <button type="button" class="btn primary" data-testid="lol-app-open-settings" onclick={onSettings}>{m.plugin_lol_source_off_open()}</button>
      {/if}
      {@render children?.()}
    </div>
  </div>
{/if}

<style>
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .actions:empty {
    display: none;
  }
</style>
