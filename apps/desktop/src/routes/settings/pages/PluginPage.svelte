<script lang="ts">
  // 插件的完整页面（ADR 038）：左侧栏「插件」组里点开。页面本身由插件给（plugins/<id>/ui，登记在 lib/pluginUi.ts）；
  // 插件关着时页面照样能打开（里面有打开它的开关）。
  import { onMount } from "svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { errText, toast } from "$lib/stores/ui";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  let { id }: { id: string } = $props();
  const plugin = $derived($pluginsStore.find((p) => p.manifest.id === id));
  const Page = $derived(PLUGIN_UI[id]?.page);
  let loaded = $state(false);

  onMount(() => {
    if ($pluginsStore.length) { loaded = true; return; }
    void listPlugins().catch((error) => toast(false, errText(error))).finally(() => (loaded = true));
  });
</script>

<div data-testid={`plugin-page`} data-plugin-id={id}>
  {#if plugin && Page}
    <Page {plugin} />
  {:else if loaded}
    <SettingsPage title={id} description={m.shell_plugin_missing()} testid="plugin-page-missing">
      <div></div>
    </SettingsPage>
  {/if}
</div>
