<script lang="ts">
  // 原神插件的设置：开关（插件、“驱动 BetterGI”，风险说明在“风险和许可”弹窗里）和 BetterGI 在哪（空着就自己找）。
  import type { PluginPageProps } from "$lib/pluginUi";
  import { pluginEnabled } from "$lib/plugins";
  import { m } from "$lib/paraglide/messages.js";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { genshinPath, saveGenshinPath } from "./genshin";

  let { plugin }: PluginPageProps = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  let draft = $state<string | null>(null);

  async function save() {
    if (draft === null) return;
    const value = draft;
    try {
      await saveGenshinPath(value);
      if (draft === value) draft = null;
    } catch (error) {
      toast(false, m.shell_toast_save_failed({ error: errText(error) }));
    }
  }
</script>

<SettingsPage title={m.plugin_genshin_options()} testid="genshin-settings">
  <SettingsSection><PluginSwitches {plugin} /></SettingsSection>
  {#if on}
    <SettingsSection>
      <SettingsRow label={m.plugin_genshin_path_label()} description={m.plugin_genshin_path_desc()}>
        {#snippet control()}
          <input class="h-9 w-72 max-w-full rounded-md border border-hairline bg-card px-3 text-[13px]" data-testid="genshin-path"
            placeholder={m.plugin_genshin_path_placeholder()} maxlength="260"
            bind:value={() => draft ?? genshinPath($settingsStore), (value) => (draft = value)}
            onchange={() => void save()} onkeydown={(event) => { if (event.key === "Enter") void save(); }} />
        {/snippet}
      </SettingsRow>
    </SettingsSection>
  {/if}
</SettingsPage>
