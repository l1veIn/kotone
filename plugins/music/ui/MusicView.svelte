<script lang="ts">
  // 她用 ui.open 打开 `ui://kotone/music/home` 时，右侧栏里的音乐页（和左侧栏的插件页同一个组件）。
  import { onMount } from "svelte";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText, toast } from "$lib/stores/ui";
  import MusicPage from "./MusicPage.svelte";
  import { PLUGIN_ID } from "./music";

  let { uri }: PluginViewProps = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === PLUGIN_ID));

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div data-testid="plugin-view-music" data-uri={uri}>
  {#if plugin}<MusicPage {plugin} />{/if}
</div>
