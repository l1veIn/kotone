<script lang="ts">
  // 插件的完整页面（ADR 038）：从「设置 → 插件」点开（左侧栏不放插件，2026-11-24 玩家定的），顶上有“返回插件”。
  // 页面本身由插件给（plugins/<id>/ui，登记在 lib/pluginUi.ts）；插件关着时页面照样能打开（里面有打开它的开关）。
  import { onMount } from "svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { ChevronLeft } from "@lucide/svelte";
  import { errText, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
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
  <div class="px-4 pt-2">
    <button type="button" data-testid="plugin-page-back"
      class="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
      onclick={() => openTarget({ page: "advanced", tab: "plugins" })}><ChevronLeft class="size-3.5" />{m.shell_plugin_back()}</button>
  </div>
  {#if plugin && Page}
    <Page {plugin} />
  {:else if loaded}
    <SettingsPage title={id} description={m.shell_plugin_missing()} testid="plugin-page-missing">
      <div></div>
    </SettingsPage>
  {/if}
</div>
