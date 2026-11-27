<script lang="ts">
  // 她用 ui.open 打开 `ui://kotone/cs2/home` 时，右侧栏里的 CS2 页面（和插件页同一个组件，不要大的边距）。
  import { onMount } from "svelte";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText, toast } from "$lib/stores/ui";
  import Cs2Page from "./Cs2Page.svelte";
  import { PLUGIN_ID } from "./cs2";

  let { uri }: PluginViewProps = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === PLUGIN_ID));

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div data-testid="plugin-view-cs2" data-uri={uri}>
  {#if plugin}<Cs2Page {plugin} embedded />{/if}
</div>
