<script lang="ts">
  // 她用 ui.open 打开 `ui://kotone/interpreter/home` 时，右侧栏里的同传页面（和插件页同一个组件，不要大标题）。
  import { onMount } from "svelte";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText, toast } from "$lib/stores/ui";
  import InterpreterPage from "./InterpreterPage.svelte";
  import { PLUGIN_ID } from "./interpreter";

  let { uri }: PluginViewProps = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === PLUGIN_ID));

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div data-testid="plugin-view-interpreter" data-uri={uri}>
  {#if plugin}<InterpreterPage {plugin} embedded />{/if}
</div>
