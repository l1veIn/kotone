<script lang="ts">
  // 英雄联盟插件的页面（ADR 039 D5）：整页就是插件的 MCP App（plugins/lol/app，fullscreen），导航在 App 里
  // （主页 · 这局 · 战绩 · 英雄 · 强度榜），在 Kotone 和 Claude 这类宿主里是同一个 App。宿主只给顶上一条栏和“设置”按钮
  // （LolSettings.svelte：数据开关、风险提示、操作客户端的偏好）。
  // 手动打开插件从召唤师主页开始。数据开关影响各页的数据，插件总开关关闭时显示设置。
  import McpAppFrame from "$lib/components/plugins/McpAppFrame.svelte";
  import { pluginEnabled, sourceEnabled } from "$lib/plugins";
  import { settingsUri, type PluginPageProps } from "$lib/pluginUi";
  import { settingsStore } from "$lib/stores/ui";
  import LolSettings from "./LolSettings.svelte";
  import { PAGE_LOL_HOME, PAGE_MATCH } from "./types";

  let { plugin, open, openSettings }: PluginPageProps = $props();
  const start = $derived.by(() => {
    if (!pluginEnabled($settingsStore, plugin.manifest)) return null;
    if (open?.params.uri === PAGE_MATCH) {
      return sourceEnabled($settingsStore, plugin.manifest, "client") ? PAGE_MATCH : null;
    }
    return PAGE_LOL_HOME;
  });

  function openPage(uri: string) {
    if (uri === settingsUri(plugin.manifest.id)) openSettings?.();
  }
</script>

<div class="min-h-full" data-testid="plugin-page-lol" data-start={start ?? ""}>
  {#if start}
    {#key open?.n}
      <McpAppFrame uri={start} displayMode="fullscreen" onOpenPage={openPage} />
    {/key}
  {:else}
    <LolSettings {plugin} />
  {/if}
</div>
