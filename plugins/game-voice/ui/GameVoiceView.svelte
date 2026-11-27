<script lang="ts">
  // ui.open 打开主页时，右侧栏使用同一张试音台的紧凑布局。
  import { onMount } from "svelte";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText, toast } from "$lib/stores/ui";
  import GameVoicePage from "./GameVoicePage.svelte";
  import { PLUGIN_ID } from "./voiceChat";

  let { uri }: PluginViewProps = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === PLUGIN_ID));

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div data-testid="plugin-view-game-voice" data-uri={uri}>
  {#if plugin}<GameVoicePage {plugin} embedded />{/if}
</div>
