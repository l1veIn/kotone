<script lang="ts">
  // 一个插件的开关（ADR 038 D3 第 2 条）：插件本身的开关 + 它的数据开关，每个数据开关带说明。
  // 设置页的插件列表、插件自己的设置页共用。开关就是设置 plugins.<id>，改了马上生效。
  // 风险提示不在这里一条条摆（ADR 043 D8）：收进插件的“风险和许可”弹窗（PluginRiskDialog），插件页第一次看到有风险的
  // 开关开着时弹一次，以后从插件「设置」里的“风险和许可”打开。
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import {
    pluginEnabled, pluginName, setPluginEnabled, setPluginSource, sourceDescription, sourceName, sourceSwitch,
    type PluginInfo,
  } from "$lib/plugins";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin, showPluginSwitch = true }: {
    plugin: PluginInfo;
    /** 显示插件本身的开关（插件页里也显示；游戏配置页的小节只要数据开关时用 false）。 */
    showPluginSwitch?: boolean;
  } = $props();

  const manifest = $derived(plugin.manifest);
  const on = $derived(pluginEnabled($settingsStore, manifest));

  async function guarded(action: () => Promise<void>) {
    try { await action(); } catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
  }
</script>

<div class="divide-y divide-hairline" data-testid={`plugin-switches-${manifest.id}`}>
  {#if showPluginSwitch}
    <div class="py-3">
      <ToggleField
        checked={on}
        label={m.system_plugins_enable({ name: pluginName(manifest) })}
        desc={m.system_plugins_enable_desc({ companion: $companionName })}
        testid={`plugin-${manifest.id}-enabled`}
        onchange={(value) => void guarded(() => setPluginEnabled(manifest.id, value))}
      />
    </div>
  {/if}
  {#each manifest.sources as source (source.id)}
    <div class="py-3" data-testid={`plugin-${manifest.id}-source-${source.id}-row`}>
      <ToggleField
        checked={sourceSwitch($settingsStore, manifest, source.id)}
        disabled={!on}
        label={sourceName(manifest, source)}
        desc={sourceDescription(manifest, source)}
        testid={`plugin-${manifest.id}-source-${source.id}`}
        onchange={(value) => void guarded(() => setPluginSource(manifest.id, source.id, value))}
      />
    </div>
  {/each}
</div>
