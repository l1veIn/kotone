<script lang="ts">
  // 一叠插件页面（ADR 038 第 2 步）：顶上一行（返回、标题、关掉），下面是最上面那一页。
  // 右侧栏（叠在 stores/pluginPages.ts）和左侧栏的插件页（叠在页面自己的状态里）都用它。
  import { ArrowLeft, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { viewOf } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import PluginView from "./PluginView.svelte";

  interface StackPage { uri: string; params: Record<string, unknown>; data: unknown; key: number }

  let { pages, onPush, onBack, onReplace, onClose, testid = "plugin-stack" }: {
    pages: StackPage[];
    onPush: (uri: string, params?: Record<string, unknown>, data?: unknown) => void;
    onBack: () => void;
    /** 最上面一页记下自己现在的样子（不重新挂载）。 */
    onReplace?: (params: Record<string, unknown>, data: unknown) => void;
    /** 没有就不显示关掉按钮（左侧栏的插件页）。 */
    onClose?: () => void;
    testid?: string;
  } = $props();

  const top = $derived(pages.length ? pages[pages.length - 1] : undefined);
  const title = $derived(top ? (viewOf(top.uri)?.view.title() ?? "") : "");
</script>

{#if top}
  <div class="flex min-h-0 flex-1 flex-col" data-testid={testid} data-uri={top.uri} data-depth={pages.length}>
    <div class="flex h-9 shrink-0 items-center gap-1 border-b border-hairline px-1.5">
      {#if pages.length > 1}
        <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_plugin_page_back()} title={m.chat_plugin_page_back()}
          data-testid={`${testid}-back`} onclick={onBack}><ArrowLeft class="size-3.5" /></Button>
      {/if}
      <p class="m-0 min-w-0 flex-1 truncate px-1 text-[13px] font-medium text-foreground" data-testid={`${testid}-title`}>{title}</p>
      {#if onClose}
        <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_plugin_page_close()} title={m.chat_plugin_page_close()}
          data-testid={`${testid}-close`} onclick={onClose}><X class="size-3.5" /></Button>
      {/if}
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-3 py-3">
      {#key top.key}
        <PluginView uri={top.uri} params={top.params} data={top.data} onOpen={onPush} {onReplace} />
      {/key}
    </div>
  </div>
{/if}
