<script lang="ts">
  // 「设置 → 插件」（ADR 038）：装在她身上的每个插件一张卡片：名字、能做什么、插件开关和数据开关（带风险提示），
  // 以及“打开”（插件自己的页面，在左侧栏「插件」组里也有）。
  import { onMount } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { listPlugins, pluginDescription, pluginEnabled, pluginName, pluginsStore } from "$lib/plugins";
  import { PLUGIN_UI } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch((error) => toast(false, errText(error)));
  });
</script>

<div class="space-y-6" data-testid="settings-plugins">
  <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground">{m.system_plugins_description({ companion: $companionName })}</p>
  {#each $pluginsStore as plugin (plugin.manifest.id)}
    {@const manifest = plugin.manifest}
    {@const Icon = PLUGIN_UI[manifest.id]?.icon}
    {@const on = pluginEnabled($settingsStore, manifest)}
    <SettingsSection testid={`settings-plugin-${manifest.id}`}>
      <div class="flex items-start justify-between gap-3 py-3.5">
        <div class="flex min-w-0 items-start gap-3">
          {#if Icon}<Icon class="mt-0.5 size-5 shrink-0 {on ? 'text-primary' : 'text-muted-foreground'}" />{/if}
          <div class="min-w-0">
            <p class="m-0 flex items-center gap-2 text-sm font-semibold text-foreground">
              {pluginName(manifest)}
              {#if !on}<span class="rounded bg-muted px-1.5 text-[10px] font-normal text-muted-foreground" data-testid={`settings-plugin-${manifest.id}-off`}>{m.system_plugins_off_badge()}</span>{/if}
            </p>
            <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{pluginDescription(manifest)}</p>
          </div>
        </div>
        {#if PLUGIN_UI[manifest.id]}
          <Button variant="outline" size="sm" class="shrink-0" data-testid={`settings-plugin-${manifest.id}-open`}
            onclick={() => openTarget({ page: "plugin", id: manifest.id })}>{m.system_plugins_open()}</Button>
        {/if}
      </div>
      <PluginSwitches {plugin} />
    </SettingsSection>
  {/each}
</div>
