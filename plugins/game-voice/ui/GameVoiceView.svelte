<script lang="ts">
  // 她用 ui.open 打开 `ui://kotone/game-voice/home` 时，右侧栏里的游戏语音页面（和左侧栏的插件页同一个组件，不要大标题）。
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
