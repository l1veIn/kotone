<script lang="ts">
  // 她用 ui.open 打开 `ui://kotone/genshin/home` 时，右侧栏里的原神页面（和插件页同一个组件，不要大边距）。
  import { onMount } from "svelte";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText, toast } from "$lib/stores/ui";
  import GenshinPage from "./GenshinPage.svelte";
  import { PLUGIN_ID } from "./genshin";

  let { uri }: PluginViewProps = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === PLUGIN_ID));

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div data-testid="plugin-view-genshin" data-uri={uri}>
  {#if plugin}<GenshinPage {plugin} embedded />{/if}
</div>
