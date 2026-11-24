<script lang="ts" module>
  /** App 里的分页（全屏时顶栏上的字）。英雄、设置还是宿主里的页面（交接第十一节第 12 条第二段再换）。 */
  export type PageId = "match" | "player" | "game" | "tier";
  export const TABS: PageId[] = ["match", "player", "tier"];
</script>

<script lang="ts">
  // 全屏每页都有的顶栏（44 px，底下一条细线）：分页是字，当前页墨色加粗 + 2 px 墨线下划线；右边是阶段牌和刷新。
  // 不放 logo 和“英雄联盟”：宿主已经写了是谁（设计第三节）。
  import type { Snippet } from "svelte";
  import { m } from "../i18n.svelte";
  import Icon from "./Icon.svelte";

  let {
    current,
    onTab,
    onRefresh,
    loading = false,
    end,
  }: {
    current: PageId;
    onTab: (page: PageId) => void;
    onRefresh?: () => void;
    loading?: boolean;
    /** 刷新按钮左边的东西（“这局”的阶段牌）。 */
    end?: Snippet;
  } = $props();

  const label = (page: PageId) =>
    page === "match" ? m.plugin_lol_page_match() : page === "player" ? m.plugin_lol_page_player() : m.plugin_lol_page_tier();
</script>

<header class="top">
  <nav class="tabs" aria-label={m.plugin_lol_name()}>
    {#each TABS as page (page)}
      <button type="button" class="tab" aria-current={current === page ? "page" : undefined} data-testid="lol-app-tab-{page}" onclick={() => onTab(page)}>
        {label(page)}
      </button>
    {/each}
  </nav>
  <div class="top-end">
    {@render end?.()}
    {#if onRefresh}
      <button type="button" class="btn" disabled={loading} data-testid="lol-app-refresh" onclick={onRefresh} title={m.plugin_lol_refresh()}>
        <Icon name="refresh" spin={loading} /><span class="label-wide">{m.plugin_lol_refresh()}</span>
      </button>
    {/if}
  </div>
</header>
