<script lang="ts">
  // 英雄联盟插件的页面（ADR 039 D5）：整页就是插件的 MCP App（plugins/lol/app，fullscreen），导航在 App 里
  // （这局 · 战绩 · 英雄 · 强度榜），在 Kotone 和 Claude 这类宿主里是同一个 App。宿主只给顶上一条栏和“设置”按钮
  // （LolSettings.svelte：数据开关、风险提示、操作客户端的偏好）。
  // App 从哪一页开始：“读客户端数据”开着从“这局”，只开了“用网上的统计”从“强度榜”（页面要的数据开关关着时宿主不给
  // 这一页的资源）；插件关着、两个开关都关着时整页就是设置（插件自己显示打开的开关）。App 里换到数据开关关着的页，
  // 那一页自己说“要先打开……”，给“去设置”（经 `ui://kotone/lol/settings` 链接，打开插件页的设置）。
  import McpAppFrame from "$lib/components/plugins/McpAppFrame.svelte";
  import { sourceEnabled } from "$lib/plugins";
  import { settingsUri, type PluginPageProps } from "$lib/pluginUi";
  import { settingsStore } from "$lib/stores/ui";
  import LolSettings from "./LolSettings.svelte";
  import { PAGE_MATCH, PAGE_TIER } from "./types";

  let { plugin, openSettings }: PluginPageProps = $props();
  const on = (source: string) => sourceEnabled($settingsStore, plugin.manifest, source);
  const start = $derived(on("client") ? PAGE_MATCH : on("stats") ? PAGE_TIER : null);

  function openPage(uri: string) {
    if (uri === settingsUri(plugin.manifest.id)) openSettings?.();
  }
</script>

<div class="min-h-full" data-testid="plugin-page-lol" data-start={start ?? ""}>
  {#if start}
    {#key start}
      <McpAppFrame uri={start} displayMode="fullscreen" onOpenPage={openPage} />
    {/key}
  {:else}
    <LolSettings {plugin} />
  {/if}
</div>
