<script lang="ts">
  // 游戏配置页里英雄联盟插件的小节（ADR 038 D3 第 9 条：插件对某个游戏的设置在插件那份里，游戏配置页下面显示它）。
  // 原来游戏配置页里的“读这局的数据”开关（ADR 035 D5）搬到这里：改的是插件的数据开关 plugins.lol.sources.client。
  // 它的风险提示不摆在这里（ADR 043 D8）：第一次在这里打开它、还没看过插件的“风险和许可”时，弹那个窗口一次。
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { openTarget } from "$lib/stores/navigation";
  import { pluginEnabled, riskUnseen, setPluginSource, sourceDescription, sourceName, sourceSwitch } from "$lib/plugins";
  import type { PluginGameSectionProps } from "$lib/pluginUi";
  import PluginRiskDialog from "$lib/components/plugins/PluginRiskDialog.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin }: PluginGameSectionProps = $props();
  const manifest = $derived(plugin.manifest);
  const client = $derived(manifest.sources.find((s) => s.id === "client"));
  const on = $derived(pluginEnabled($settingsStore, manifest));

  let riskOpen = $state(false);

  async function toggle(value: boolean) {
    try {
      await setPluginSource(manifest.id, "client", value);
      // 打开了有风险的开关、这一版的“风险和许可”还没看过：弹一次（只是提示，开关已经照你选的存了）
      if (value && client?.risk && riskUnseen($settingsStore, manifest)) riskOpen = true;
    } catch (error) { toast(false, errText(error)); }
  }
</script>

<div class="space-y-2 rounded-lg border border-hairline bg-background/40 p-3.5" data-testid="game-profile-lol-data">
  <div class="flex items-start justify-between gap-3">
    <div class="min-w-0">
      <p class="m-0 text-[13px] font-medium text-foreground">{m.plugin_lol_game_section_title()}</p>
      <p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">{m.plugin_lol_game_section_desc()}</p>
    </div>
    <Button variant="outline" size="sm" class="shrink-0" data-testid="game-profile-lol-open"
      onclick={() => openTarget({ page: "plugin", id: manifest.id })}>{m.plugin_lol_game_section_open()}</Button>
  </div>
  {#if client}
    <ToggleField
      checked={sourceSwitch($settingsStore, manifest, client.id)}
      disabled={!on}
      label={sourceName(manifest, client)}
      desc={sourceDescription(manifest, client)}
      testid="profile-lol-data"
      onchange={(value) => void toggle(value)}
    />
    {#if !on}<p class="m-0 text-xs text-muted-foreground" data-testid="game-profile-lol-plugin-off">{m.plugin_lol_game_section_plugin_off()}</p>{/if}
  {/if}
</div>

<PluginRiskDialog {plugin} bind:open={riskOpen} />
