<script lang="ts">
  // 一个插件的 App（ADR 038，照 MCP Apps）：从左侧栏「插件」的卡片点开，占满内容区。宿主只给顶上一条很薄的栏
  // （“← 插件”和插件名），下面整块是插件自己的界面、自己的设计（插件关着时它自己显示打开的开关）。
  // App 的位置是通用的：现在放内置插件的页面（plugins/<id>/ui，登记在 lib/pluginUi.ts）；以后的第三方插件
  // （别的仓库的 MCP App）在这里用沙盒 iframe 显示它的 `ui://` 资源，宿主不给它套样式。
  import { onMount } from "svelte";
  import { ChevronLeft } from "@lucide/svelte";
  import { errText, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { listPlugins, pluginName, pluginsStore } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  let { id }: { id: string } = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === id));
  const ui = $derived(PLUGIN_UI[id]);
  const Page = $derived(ui?.page);
  const Icon = $derived(ui?.icon);
  let loaded = $state(false);

  onMount(() => {
    if ($pluginsStore.length) { loaded = true; return; }
    void listPlugins().catch((error) => toast(false, errText(error))).finally(() => (loaded = true));
  });
</script>

<div class="flex min-h-full flex-col" data-testid="plugin-page" data-plugin-id={id}>
  <!-- 宿主的栏：只有返回和名字 -->
  <div class="flex h-9 shrink-0 items-center gap-2 border-b border-hairline px-3" data-testid="plugin-app-bar">
    <button type="button" data-testid="plugin-page-back" title={m.shell_plugin_back()} aria-label={m.shell_plugin_back()}
      class="inline-flex items-center gap-0.5 rounded-md px-1.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
      onclick={() => openTarget({ page: "plugins" })}><ChevronLeft class="size-3.5" />{m.shell_sidebar_plugins()}</button>
    <span class="text-hairline">/</span>
    {#if Icon}<Icon class="size-3.5 text-muted-foreground" />{/if}
    <span class="truncate text-xs font-medium text-foreground" data-testid="plugin-app-name">{plugin ? pluginName(plugin.manifest) : id}</span>
  </div>
  <!-- App 的位置：插件自己的界面，宿主不加边距、标题、样式 -->
  <div class="relative min-h-0 flex-1" data-testid="plugin-app">
    {#if plugin && Page}
      <Page {plugin} />
    {:else if loaded}
      <p class="m-0 px-8 py-10 text-sm text-muted-foreground" data-testid="plugin-page-missing">{m.shell_plugin_missing()}</p>
    {/if}
  </div>
</div>
