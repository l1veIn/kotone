<script lang="ts">
  // 一组设置：组标题 + 说明在卡片外，卡片里是一行一行的设置（行间细分隔线）。
  // 参照 DeepSeek Harness 的 settings card（0.5px 细边、20px 圆角、行间分隔）。
  import type { Snippet } from "svelte";

  let { title = "", description = "", actions, children, flush = false, testid, id }: {
    title?: string;
    description?: string;
    actions?: Snippet;
    children: Snippet;
    /** 卡片不留内边距（内容自己排，例如列表）。 */
    flush?: boolean;
    testid?: string;
    id?: string;
  } = $props();
  const headingId = $derived(id ?? (title ? `section-${title}` : undefined));
</script>

<section class="min-w-0" aria-labelledby={title ? headingId : undefined} data-testid={testid}>
  {#if title || actions}
    <div class="mb-2.5 flex items-end justify-between gap-3 px-1">
      <div class="min-w-0">
        {#if title}<h2 id={headingId} class="m-0 text-sm font-semibold text-foreground">{title}</h2>{/if}
        {#if description}<p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{description}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
    </div>
  {/if}
  <div class="rounded-2xl border border-hairline bg-card/70 {flush ? '' : 'px-4'} [&>*+*]:border-t [&>*+*]:border-hairline">
    {@render children()}
  </div>
</section>
