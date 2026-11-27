<script lang="ts">
  // CS2 插件的设置：插件开关（宿主的 PluginSwitches）；下面是宿主画的“推送”。写不写游戏的配置文件在插件页上（看得到写在哪）。
  import type { PluginPageProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";

  let { plugin }: PluginPageProps = $props();
</script>

<SettingsPage title={m.plugin_cs2_options()} testid="cs2-settings">
  <SettingsSection><PluginSwitches {plugin} /></SettingsSection>
</SettingsPage>
