<script lang="ts">
  // 设置页的统一骨架（参照 DeepSeek Harness 设置页：居中的内容列、页标题 + 一句说明、下面是若干设置卡）。
  import type { Snippet } from "svelte";

  let { title, description = "", actions, children, testid, embedded = false }: {
    title: string;
    description?: string;
    /** 标题右侧的操作（按钮等）。 */
    actions?: Snippet;
    children: Snippet;
    /** 已不用：所有设置页内容宽度一致（ADR 023）。保留参数以兼容旧调用。 */
    wide?: boolean;
    testid?: string;
    /** 嵌在语音输入子板块里（ADR 028）：上面已有子板块标题和分页，这里不再重复页标题。 */
    embedded?: boolean;
  } = $props();
</script>

<div class="ui-scope mx-auto w-full px-6 pb-10 text-foreground max-w-3xl {embedded ? 'pt-5' : 'pt-7'}" data-testid={testid}>
  {#if !embedded || description || actions}
    <header class="mb-6 flex flex-wrap items-end justify-between gap-x-4 gap-y-2">
      <div class="min-w-0">
        {#if embedded}<h2 class="sr-only">{title}</h2>{:else}<h1 class="m-0 text-xl font-semibold tracking-tight">{title}</h1>{/if}
        {#if description}<p class="{embedded ? 'm-0' : 'mt-1.5 mb-0'} max-w-2xl text-[13px] leading-relaxed text-muted-foreground">{description}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class="space-y-6">
    {@render children()}
  </div>
</div>
