<script lang="ts">
  // 一个插件的开关（ADR 038 D3 第 2 条）：插件本身的开关 + 它的数据开关，每个数据开关带说明和风险提示。
  // 设置页的插件列表、插件自己的设置页共用。开关就是设置 plugins.<id>，改了马上生效。
  import { TriangleAlert } from "@lucide/svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import {
    pluginEnabled, pluginName, setPluginEnabled, setPluginSource, sourceDescription, sourceName, sourceRisk, sourceSwitch,
    type PluginInfo,
  } from "$lib/plugins";
  import { openExternal } from "$lib/ipc";
  import { m } from "$lib/paraglide/messages.js";

  /** 风险提示里的 GitHub 地址（例如替玩家问 OP.GG 的 issue）做成能点的链接，用系统浏览器打开。 */
  const LINK = /(github\.com\/[\w.\-/]+[\w/])/;
  function segments(text: string): { text: string; link: boolean }[] {
    return text.split(LINK).filter(Boolean).map((part) => ({ text: part, link: LINK.test(part) }));
  }

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
    {@const risk = sourceRisk(manifest, source)}
    <div class="space-y-2 py-3" data-testid={`plugin-${manifest.id}-source-${source.id}-row`}>
      <ToggleField
        checked={sourceSwitch($settingsStore, manifest, source.id)}
        disabled={!on}
        label={sourceName(manifest, source)}
        desc={sourceDescription(manifest, source)}
        testid={`plugin-${manifest.id}-source-${source.id}`}
        onchange={(value) => void guarded(() => setPluginSource(manifest.id, source.id, value))}
      />
      {#if risk}
        <p class="m-0 flex items-start gap-1.5 rounded-lg bg-amber-500/8 px-2.5 py-2 text-xs leading-relaxed text-amber-700 dark:text-amber-200/90"
          data-testid={`plugin-${manifest.id}-source-${source.id}-risk`}>
          <TriangleAlert class="mt-0.5 size-3.5 shrink-0" /><span>{#each segments(risk) as part, i (i)}{#if part.link}<button type="button"
            class="underline underline-offset-2 hover:text-foreground" data-testid={`plugin-${manifest.id}-source-${source.id}-link`}
            onclick={() => void openExternal(`https://${part.text}`)}>{part.text}</button>{:else}{part.text}{/if}{/each}</span>
        </p>
      {/if}
    </div>
  {/each}
</div>
