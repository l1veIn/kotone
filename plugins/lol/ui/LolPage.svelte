<script lang="ts">
  // 英雄联盟插件的页面（ADR 038）：标题 + 分页。这局（`lol.scout`）、战绩（`lol.player`，能搜别人）、设置（数据开关和
  // 风险提示）。这局和战绩里能一路点进去（这局 → 某个人 → 某一局），每个分页自己记着点到哪了，能返回。
  // 英雄、海克斯在后面几步加（docs/plugin-refactor.md）。
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import PluginStack from "$lib/components/plugins/PluginStack.svelte";
  import { companionName } from "$lib/stores/companion";
  import { pluginDescription, pluginName, sourceEnabled } from "$lib/plugins";
  import { settingsStore } from "$lib/stores/ui";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_MATCH, PAGE_PLAYER } from "./types";

  let { plugin }: PluginPageProps = $props();
  type Tab = "match" | "player" | "settings";
  const tabs: { id: Tab; label: string }[] = [
    { id: "match", label: m.plugin_lol_page_match() },
    { id: "player", label: m.plugin_lol_page_player() },
    { id: "settings", label: m.plugin_lol_tab_settings() },
  ];
  // 插件或“读客户端数据”关着时先停在设置（这局、战绩要它）
  const startTab = (): Tab => (sourceEnabled($settingsStore, plugin.manifest, "client") ? "match" : "settings");
  let tab = $state<Tab>(startTab());

  interface StackPage { uri: string; params: Record<string, unknown>; data: unknown; key: number }
  let counter = 0;
  const root = (uri: string): StackPage[] => [{ uri, params: {}, data: null, key: ++counter }];
  /** 每个分页一叠（这局、战绩各自点进去的页面）。 */
  let stacks = $state<Record<"match" | "player", StackPage[]>>({ match: root(PAGE_MATCH), player: root(PAGE_PLAYER) });

  function push(which: "match" | "player", uri: string, params?: Record<string, unknown>, data?: unknown) {
    stacks[which] = [...stacks[which], { uri, params: params ?? {}, data: data ?? null, key: ++counter }];
  }
  function back(which: "match" | "player") {
    if (stacks[which].length > 1) stacks[which] = stacks[which].slice(0, -1);
  }
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
  {:else}
    {@const which = tab}
    <div class="flex min-h-[24rem] flex-col overflow-hidden rounded-xl border border-hairline bg-card/40">
      <PluginStack pages={stacks[which]} onPush={(uri, params, data) => push(which, uri, params, data)} onBack={() => back(which)}
        testid={`plugin-lol-${which}`} />
    </div>
  {/if}
</SettingsPage>
