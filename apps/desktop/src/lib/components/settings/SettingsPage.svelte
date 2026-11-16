<script lang="ts">
  // 设置页的统一骨架（参照 DeepSeek Harness 设置页：居中的内容列、页标题 + 一句说明、下面是若干设置卡）。
  import type { Snippet } from "svelte";

  let { title, description = "", actions, children, wide = false, testid }: {
    title: string;
    description?: string;
    /** 标题右侧的操作（按钮等）。 */
    actions?: Snippet;
    children: Snippet;
    /** 宽一些的内容列（表格、编辑器较多的页面）。 */
    wide?: boolean;
    testid?: string;
  } = $props();
</script>

<div class="ui-scope mx-auto w-full px-6 pt-7 pb-10 text-foreground {wide ? 'max-w-5xl' : 'max-w-3xl'}" data-testid={testid}>
  <header class="mb-6 flex flex-wrap items-end justify-between gap-x-4 gap-y-2">
    <div class="min-w-0">
      <h1 class="m-0 text-xl font-semibold tracking-tight">{title}</h1>
      {#if description}<p class="mt-1.5 mb-0 max-w-2xl text-[13px] leading-relaxed text-muted-foreground">{description}</p>{/if}
    </div>
    {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
  </header>
  <div class="space-y-6">
    {@render children()}
  </div>
</div>
