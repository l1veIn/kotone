<script lang="ts">
  import { listModels, type ModelInfo } from "$lib/ipc";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { pluginEnabled } from "$lib/plugins";
  import { m } from "$lib/paraglide/messages.js";
  import { openTarget } from "$lib/stores/navigation";
  import { runtimeStore } from "$lib/stores/runtime";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { interpreterPrefs, saveInterpreter } from "./interpreter";

  let { plugin }: PluginPageProps = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const rt = $derived($runtimeStore);
  const display = $derived(interpreterPrefs($settingsStore, []).display);
  const options = $derived([
    { value: "bubble", label: m.plugin_interpreter_display_bubble() },
    { value: "danmaku", label: m.plugin_interpreter_display_danmaku() },
  ]);
  let models = $state<ModelInfo[]>([]);
  $effect(() => {
    if (rt?.listenModelId) void listModels().then((list) => { models = list; }).catch(() => {});
  });
  const recognizerName = $derived((models.find((model) => model.id === rt?.listenModelId)?.displayName ?? rt?.listenModelId ?? "").replace(/（.*$/, ""));

  async function setDisplay(value: string) {
    try { await saveInterpreter({ display: value === "danmaku" ? "danmaku" : "bubble" }); }
    catch (error) { toast(false, m.shell_toast_save_failed({ error: errText(error) })); }
  }
</script>

<SettingsPage title={m.plugin_interpreter_options()} testid="interpreter-settings">
  <SettingsSection><PluginSwitches {plugin} /></SettingsSection>
  {#if on}
    <SettingsSection>
      <SettingsRow label={m.plugin_interpreter_display_label()} description={m.plugin_interpreter_display_desc()}>
        {#snippet control()}
          <div class="w-48 max-w-full"><SelectField ariaLabel={m.plugin_interpreter_display_label()} testid="interpreter-display"
            value={display} {options} onchange={setDisplay} /></div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.plugin_interpreter_recognizer_label()} description={rt?.listenModelId
        ? (rt.listenOnline ? m.plugin_interpreter_recognizer_online_note() : m.plugin_interpreter_recognizer_local_note())
        : (rt?.listenError ?? "")}>
        {#snippet control()}
          <div class="flex max-w-full flex-wrap items-center gap-3">
            <span class="text-[13px]" data-testid="interpreter-recognizer" data-online={rt?.listenOnline ? "true" : "false"}>
              {rt?.listenModelId ? (rt.listenOnline ? m.plugin_interpreter_recognizer_online({ name: recognizerName }) : m.plugin_interpreter_recognizer_local({ name: recognizerName })) : m.plugin_interpreter_recognizer_missing()}
            </span>
            <button class="text-xs text-primary hover:underline" data-testid="interpreter-open-models"
              onclick={() => openTarget({ page: "advanced", tab: "models", anchor: "models-asr" })}>{m.plugin_interpreter_recognizer_open_models()}</button>
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>
  {/if}
</SettingsPage>
