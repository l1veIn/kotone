<script lang="ts">
  // 英雄联盟插件的页面（ADR 038 第 1 步）：标题 + 分页。第 1 步只有“设置”（数据开关和风险提示）；
  // 这局、战绩、英雄、海克斯在后面几步加（docs/plugin-refactor.md）。
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { companionName } from "$lib/stores/companion";
  import { pluginDescription, pluginName } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin }: PluginPageProps = $props();
  type Tab = "settings";
  const tabs: { id: Tab; label: string }[] = [{ id: "settings", label: m.plugin_lol_tab_settings() }];
  let tab = $state<Tab>("settings");
</script>

<SettingsPage title={pluginName(plugin.manifest)} description={pluginDescription(plugin.manifest)} testid="plugin-page-lol">
  <nav class="-mt-2 flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1" aria-label={pluginName(plugin.manifest)}>
    {#each tabs as item (item.id)}
      {@const active = tab === item.id}
      <button type="button" data-testid={`plugin-lol-tab-${item.id}`} aria-current={active ? "page" : undefined}
        class="h-7 rounded-lg px-3 text-xs font-medium transition-colors {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => (tab = item.id)}>{item.label}</button>
    {/each}
  </nav>
  {#if tab === "settings"}
    <SettingsSection title={m.plugin_lol_tab_settings()} description={m.plugin_lol_settings_intro({ companion: $companionName })} testid="plugin-lol-settings">
      <PluginSwitches {plugin} />
    </SettingsSection>
  {/if}
</SettingsPage>
