<script lang="ts">
  // 设置卡里的一行：左边标题与说明，右边控件；stacked 时控件在下方占满一行。
  import type { Snippet } from "svelte";

  let { label, description = "", control, children, stacked = false, for: forId, testid }: {
    label: string;
    description?: string | Snippet;
    /** 右侧（或下方）的控件。 */
    control?: Snippet;
    /** 额外内容（显示在行的下方）。 */
    children?: Snippet;
    stacked?: boolean;
    /** 标题关联的控件 id。 */
    for?: string;
    testid?: string;
  } = $props();
</script>

<div class="py-3.5" data-testid={testid}>
  <div class="flex {stacked ? 'flex-col gap-2.5' : 'flex-wrap items-center justify-between gap-x-6 gap-y-2'}">
    <div class="min-w-0 {stacked ? '' : 'max-w-md flex-1'}">
      {#if forId}<label for={forId} class="block text-[13px] font-medium text-foreground">{label}</label>
      {:else}<p class="m-0 text-[13px] font-medium text-foreground">{label}</p>{/if}
      {#if typeof description === "string" && description}<p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">{description}</p>
      {:else if typeof description === "function"}<div class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{@render description()}</div>{/if}
    </div>
    {#if control}<div class="min-w-0 {stacked ? 'w-full' : 'shrink-0'}">{@render control()}</div>{/if}
  </div>
  {#if children}<div class="mt-3">{@render children()}</div>{/if}
</div>
