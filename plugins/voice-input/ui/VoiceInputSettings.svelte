<script lang="ts">
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import type { PluginPageProps } from "$lib/pluginUi";
  let { plugin }: PluginPageProps = $props();
</script>

<div class="ui-scope mx-auto w-full max-w-3xl px-6 pt-6 pb-8 text-foreground">
  <SettingsSection><PluginSwitches {plugin} /></SettingsSection>
</div>
