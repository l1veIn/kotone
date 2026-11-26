<script lang="ts">
  // 一个插件的 App（ADR 038，照 MCP Apps）：从左侧栏「插件」的卡片点开，占满内容区。宿主只给顶上一条很薄的栏
  // （“← 插件”和插件名），下面整块是插件自己的界面、自己的设计（插件关着时它自己显示打开的开关）。
  // 插件登记了设置（`PluginUi.settings`）或者清单里有内置推送（`[[pushes]]`）时，栏的右边有一个“设置”按钮：在同一处打开
  // 插件的设置（数据开关、风险提示、插件自己的偏好），下面是宿主画的“推送”一节（ADR 043 D4：每条一个开关，宿主执行）；
  // 再点一次或点插件名回到 App；App 一直挂着（iframe 不重新加载、点到哪还在哪）。
  // 风险和许可（ADR 043 D8）：插件有风险提示或许可说明时，设置里栏上多一个小链接“风险和许可”，打开一个弹窗（PluginRiskDialog）；
  // 这一版还没看过、又看到它有风险的开关开着（默认就开着的，或者刚在这里打开的）时自己弹一次。只是提示，不动开关。
  // App 的位置是通用的：现在放内置插件的页面（plugins/<id>/ui，登记在 lib/pluginUi.ts）；以后的第三方插件
  // （别的仓库的 MCP App）在这里用沙盒 iframe 显示它的 `ui://` 资源，宿主不给它套样式。
  import { onMount } from "svelte";
  import { ChevronLeft, Settings2, ShieldAlert } from "@lucide/svelte";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { hasRiskNotes, listPlugins, pluginName, pluginsStore, riskUnseen, riskyOn } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import PluginPushes from "$lib/components/plugins/PluginPushes.svelte";
  import PluginRiskDialog from "$lib/components/plugins/PluginRiskDialog.svelte";
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

  /** 有“风险和许可”要看（数据开关的风险提示、许可和出处）。 */
  const hasRisk = $derived(!!plugin && hasRiskNotes(plugin.manifest));
  let riskOpen = $state(false);
  /** 这次打开插件页已经自己弹过一次（记“看过”没存上时也不再弹第二次）。 */
  let riskShown = false;
  // 第一次看到有风险的开关开着（默认开着的、刚打开的；设置读到了才算）：弹一次，弹窗自己记下看过
  $effect(() => {
    if (riskShown || !plugin || !$settingsStore) return;
    if (riskUnseen($settingsStore, plugin.manifest) && riskyOn($settingsStore, plugin.manifest)) {
      riskShown = true;
      riskOpen = true;
    }
  });
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
    <div class="ml-auto flex items-center gap-1">
      <!-- 风险和许可：在设置里（插件没有设置时一直在栏上） -->
      {#if plugin && hasRisk && (showSettings || !hasSettings)}
        <button type="button" data-testid="plugin-risk-link"
          class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
          onclick={() => (riskOpen = true)}><ShieldAlert class="size-3.5" />{m.system_plugins_risk_link()}</button>
      {/if}
      {#if plugin && hasSettings}
        <button type="button" data-testid="plugin-settings-button" aria-pressed={showSettings}
          title={showSettings ? m.shell_plugin_settings_close() : m.shell_plugin_settings()}
          class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs transition-colors hover:bg-muted/60
            {showSettings ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => (showSettings = !showSettings)}><Settings2 class="size-3.5" />{m.shell_plugin_settings()}</button>
      {/if}
    </div>
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

{#if plugin && hasRisk}<PluginRiskDialog {plugin} bind:open={riskOpen} />{/if}
