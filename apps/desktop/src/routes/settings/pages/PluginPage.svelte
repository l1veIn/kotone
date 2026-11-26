<script lang="ts">
  // 一个插件的 App（ADR 038，照 MCP Apps）：从左侧栏「插件」的卡片点开，占满内容区。宿主只给顶上一条很薄的栏
  // （“← 插件”和插件名），下面整块是插件自己的界面、自己的设计（插件关着时它自己显示打开的开关）。
  // 插件登记了设置（`PluginUi.settings`）或者清单里有内置推送（`[[pushes]]`）时，栏的右边有一个“设置”按钮：在同一处打开
  // 插件的设置（数据开关、风险提示、插件自己的偏好），下面是宿主画的“推送”一节（ADR 043 D4：每条一个开关，宿主执行）；
  // 再点一次或点插件名回到 App；App 一直挂着（iframe 不重新加载、点到哪还在哪）。
  // App 的位置是通用的：现在放内置插件的页面（plugins/<id>/ui，登记在 lib/pluginUi.ts）；以后的第三方插件
  // （别的仓库的 MCP App）在这里用沙盒 iframe 显示它的 `ui://` 资源，宿主不给它套样式。
  import { onMount } from "svelte";
  import { ChevronLeft, Settings2 } from "@lucide/svelte";
  import { errText, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { listPlugins, pluginName, pluginsStore } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import PluginPushes from "$lib/components/plugins/PluginPushes.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { id, open = null }: { id: string; open?: { params: Record<string, unknown>; n: number } | null } = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === id));
  const ui = $derived(PLUGIN_UI[id]);
  const Page = $derived(ui?.page);
  const Settings = $derived(ui?.settings);
  const Icon = $derived(ui?.icon);
  /** 有内置推送：设置里有宿主画的“推送”一节。 */
  const hasPushes = $derived((plugin?.manifest.pushes?.length ?? 0) > 0);
  const hasSettings = $derived(!!Settings || hasPushes);
  let loaded = $state(false);
  /** 正在看插件的设置（不是 App）。 */
  let showSettings = $state(false);

  onMount(() => {
    if ($pluginsStore.length) { loaded = true; return; }
    void listPlugins().catch((error) => toast(false, errText(error))).finally(() => (loaded = true));
  });

  // 「去看看」的位置、App 里的“去设置”（`openTarget({ page: "plugin", id, params: { view: "settings" } })`）
  $effect(() => {
    if (open) showSettings = open.params.view === "settings" && hasSettings;
  });
  const openSettings = () => (showSettings = true);
</script>

<div class="flex min-h-full flex-col" data-testid="plugin-page" data-plugin-id={id} data-view={showSettings ? "settings" : "app"}>
  <!-- 宿主的栏：返回、名字；插件有设置时右边一个“设置” -->
  <div class="flex h-9 shrink-0 items-center gap-2 border-b border-hairline px-3" data-testid="plugin-app-bar">
    <button type="button" data-testid="plugin-page-back" title={m.shell_plugin_back()} aria-label={m.shell_plugin_back()}
      class="inline-flex items-center gap-0.5 rounded-md px-1.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
      onclick={() => openTarget({ page: "plugins" })}><ChevronLeft class="size-3.5" />{m.shell_sidebar_plugins()}</button>
    <span class="text-hairline">/</span>
    {#if Icon}<Icon class="size-3.5 text-muted-foreground" />{/if}
    {#if showSettings}
      <button type="button" class="truncate rounded-md px-1 py-0.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
        data-testid="plugin-app-name" onclick={() => (showSettings = false)}>{plugin ? pluginName(plugin.manifest) : id}</button>
      <span class="text-hairline">/</span>
      <span class="truncate text-xs font-medium text-foreground">{m.shell_plugin_settings()}</span>
    {:else}
      <span class="truncate text-xs font-medium text-foreground" data-testid="plugin-app-name">{plugin ? pluginName(plugin.manifest) : id}</span>
    {/if}
    {#if plugin && hasSettings}
      <button type="button" data-testid="plugin-settings-button" aria-pressed={showSettings}
        title={showSettings ? m.shell_plugin_settings_close() : m.shell_plugin_settings()}
        class="ml-auto inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs transition-colors hover:bg-muted/60
          {showSettings ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => (showSettings = !showSettings)}><Settings2 class="size-3.5" />{m.shell_plugin_settings()}</button>
    {/if}
  </div>
  <!-- App 的位置：插件自己的界面，宿主不加边距、标题、样式 -->
  <div class="relative min-h-0 flex-1" data-testid="plugin-app">
    {#if plugin && Page}
      <div class:hidden={showSettings} class="min-h-full">
        <Page {plugin} {open} openSettings={hasSettings ? openSettings : undefined} />
      </div>
      {#if showSettings && hasSettings}
        <div data-testid="plugin-settings">
          {#if Settings}<Settings {plugin} {open} />{/if}
          {#if hasPushes}
            <!-- 宿主画的“推送”（和设置页同一列宽） -->
            <div class="ui-scope mx-auto w-full max-w-3xl px-6 pb-10 text-foreground {Settings ? '-mt-4' : 'pt-7'}">
              <PluginPushes {plugin} />
            </div>
          {/if}
        </div>
      {/if}
    {:else if loaded}
      <p class="m-0 px-8 py-10 text-sm text-muted-foreground" data-testid="plugin-page-missing">{m.shell_plugin_missing()}</p>
    {/if}
  </div>
</div>
