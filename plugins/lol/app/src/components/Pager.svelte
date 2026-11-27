<script lang="ts">
  // 战绩的翻页（主页的近期战绩、战绩页）：上一页 · 第几页 · 下一页；翻到头时左边写一句客户端给了几局
  // （客户端按人只给一段战绩，plugins/lol/src/watcher/history_page.rs）。
  import Icon from "./Icon.svelte";
  import { m } from "../i18n.svelte";

  let { id, label, page, canPrev, canNext, clientGames = 0, onPrev, onNext }: {
    /** testid 前缀：`lol-<id>-prev` / `-page` / `-next` / `-end`。 */
    id: string;
    label: string;
    page: number;
    canPrev: boolean;
    canNext: boolean;
    /** 翻到头时客户端一共给了几局（0 = 不写）。 */
    clientGames?: number;
    onPrev: () => void;
    onNext: () => void;
  } = $props();
</script>

<nav class="pager" aria-label={label}>
  {#if clientGames > 0}<span class="end" data-testid="lol-{id}-end">{m.plugin_lol_history_client_limit({ count: clientGames })}</span>{/if}
  <button type="button" class="btn" data-testid="lol-{id}-prev" disabled={!canPrev} onclick={onPrev}>
    <Icon name="back" />{m.plugin_lol_history_prev()}
  </button>
  <span class="num" data-testid="lol-{id}-page" aria-live="polite">{m.plugin_lol_history_page({ page })}</span>
  <button type="button" class="btn" data-testid="lol-{id}-next" disabled={!canNext} onclick={onNext}>
    {m.plugin_lol_history_next()}<span class="forward"><Icon name="back" /></span>
  </button>
</nav>

<style>
  .pager, .forward { display: flex; align-items: center; }
  .pager { justify-content: flex-end; flex-wrap: wrap; gap: 8px 14px; padding-top: 10px; color: var(--lol-text-2); font-size: var(--lol-fs-xs); }
  .end { margin-inline-end: auto; }
  .forward { transform: rotate(180deg); }
</style>
