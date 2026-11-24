<script lang="ts">
  // 英雄联盟插件的页面（ADR 038）：标题 + 分页。这局（`lol.scout`）、战绩（`lol.player`，能搜别人）——要“读客户端数据”；
  // 英雄（`lol.champion`，能搜、切峡谷 / 海斗、切分路）、强度榜（`lol.tier`）——要“用网上的统计”；设置（数据开关和风险提示）。
  // 每个分页里能一路点进去（这局 → 某个人 → 某一局；强度榜 → 某个英雄 → 克制它的英雄），各自记着点到哪了，能返回。
  // 分页要的数据开关关着时，只写一句并给个“去设置”。
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import PluginStack from "$lib/components/plugins/PluginStack.svelte";
  import { Button } from "$lib/components/ui/button";
  import { companionName } from "$lib/stores/companion";
  import { pluginDescription, pluginName, sourceEnabled, sourceName } from "$lib/plugins";
  import { settingsStore } from "$lib/stores/ui";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import { PAGE_CHAMPION, PAGE_MATCH, PAGE_PLAYER, PAGE_TIER } from "./types";

  let { plugin }: PluginPageProps = $props();
  type StackTab = "match" | "player" | "champion" | "tier";
  type Tab = StackTab | "settings";
  /** 分页、它的第一页、它要的数据开关。 */
  const tabs: { id: Tab; label: string; uri?: string; needs?: string }[] = [
    { id: "match", label: m.plugin_lol_page_match(), uri: PAGE_MATCH, needs: "client" },
    { id: "player", label: m.plugin_lol_page_player(), uri: PAGE_PLAYER, needs: "client" },
    { id: "champion", label: m.plugin_lol_page_champion(), uri: PAGE_CHAMPION, needs: "stats" },
    { id: "tier", label: m.plugin_lol_page_tier(), uri: PAGE_TIER, needs: "stats" },
    { id: "settings", label: m.plugin_lol_tab_settings() },
  ];
  const on = (source: string) => sourceEnabled($settingsStore, plugin.manifest, source);
  // 先停在第一个用得了的分页（都用不了就是设置）
  const startTab = (): Tab => tabs.find((t) => t.needs && on(t.needs))?.id ?? "settings";
  let tab = $state<Tab>(startTab());
  const current = $derived(tabs.find((t) => t.id === tab) ?? tabs[tabs.length - 1]);
  const blocked = $derived(current.needs && !on(current.needs) ? current.needs : null);

  interface StackPage { uri: string; params: Record<string, unknown>; data: unknown; key: number }
  let counter = 0;
  const root = (uri: string): StackPage[] => [{ uri, params: {}, data: null, key: ++counter }];
  /** 每个分页一叠（各自点进去的页面）。 */
  let stacks = $state<Record<StackTab, StackPage[]>>({
    match: root(PAGE_MATCH), player: root(PAGE_PLAYER), champion: root(PAGE_CHAMPION), tier: root(PAGE_TIER),
  });

  function push(which: StackTab, uri: string, params?: Record<string, unknown>, data?: unknown) {
    stacks[which] = [...stacks[which], { uri, params: params ?? {}, data: data ?? null, key: ++counter }];
  }
  /** 最上面一页记下自己现在的样子（例如搜过的英雄），返回到它时照这样显示。 */
  function replace(which: StackTab, params: Record<string, unknown>, data: unknown) {
    const stack = stacks[which];
    stacks[which] = stack.map((p, i) => (i === stack.length - 1 ? { ...p, params, data } : p));
  }
  function back(which: StackTab) {
    if (stacks[which].length > 1) stacks[which] = stacks[which].slice(0, -1);
  }
  const sourceLabel = (id: string) => {
    const spec = plugin.manifest.sources.find((s) => s.id === id);
    return spec ? sourceName(plugin.manifest, spec) : id;
  };
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
  {:else if blocked}
    <div class="flex flex-wrap items-center gap-2 rounded-xl border border-hairline bg-card/40 px-4 py-3 text-xs text-muted-foreground" data-testid="plugin-lol-source-off">
      <span>{m.plugin_lol_source_off({ source: sourceLabel(blocked) })}</span>
      <Button size="sm" variant="outline" class="h-7 px-2.5 text-xs" onclick={() => (tab = "settings")}>{m.plugin_lol_source_off_open()}</Button>
    </div>
  {:else}
    {@const which = tab as StackTab}
    <div class="flex min-h-[24rem] flex-col overflow-hidden rounded-xl border border-hairline bg-card/40">
      <PluginStack pages={stacks[which]} onPush={(uri, params, data) => push(which, uri, params, data)} onBack={() => back(which)}
        onReplace={(params, data) => replace(which, params, data)} testid={`plugin-lol-${which}`} />
    </div>
  {/if}
</SettingsPage>
