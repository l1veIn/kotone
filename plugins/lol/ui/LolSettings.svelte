<script lang="ts">
  // 英雄联盟插件的设置（宿主的页面；插件页顶栏的“设置”打开）：数据开关和它们的风险提示、“操作客户端”的偏好
  // （自动接受、自动选 / 禁英雄，照 LeagueAkari）。App（这局、战绩、英雄、强度榜）里不放设置：在 Claude 这类宿主里
  // 没有 Kotone 的设置，App 只在页脚写风险提示。
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { companionName } from "$lib/stores/companion";
  import { pluginName } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";
  import ControlSettings from "./ControlSettings.svelte";

  let { plugin }: PluginPageProps = $props();
</script>

<SettingsPage title={pluginName(plugin.manifest)} description={m.plugin_lol_settings_intro({ companion: $companionName })} testid="plugin-lol-settings-page">
  <SettingsSection title={m.plugin_lol_tab_settings()} testid="plugin-lol-settings">
    <PluginSwitches {plugin} />
  </SettingsSection>
  <!-- 操作客户端的偏好（第 3.5 步，照 LeagueAkari）：数据开关 control 开着才能改 -->
  <section class="min-w-0 space-y-2.5" data-testid="plugin-lol-control-section">
    <div class="px-1">
      <h2 class="m-0 text-sm font-semibold text-foreground">{m.plugin_lol_control_title()}</h2>
      <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{m.plugin_lol_control_intro()}</p>
    </div>
    <ControlSettings {plugin} />
  </section>
</SettingsPage>
