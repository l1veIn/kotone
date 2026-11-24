<script lang="ts">
  // 同传插件的页面（ADR 021、038）：设好听哪个程序、译成什么、怎么显示，点“开始”就等于你在聊天里对她说了那句话，
  // 她交给后台去听、去翻；“停止”就是对她说“停止同传”。下面一行是她的耳朵现在在听哪些程序（开着页面时每 2 秒读一次）。
  // 左侧栏「设置 → 插件 → 同传」和她 `ui.open` 打开的右侧栏页面共用（右侧栏里不重复页标题）。
  // 插件开关是 `plugins.interpreter.enabled`；关着时只显示开关和一句说明。
  import { onMount } from "svelte";
  import { Ear, EarOff, LoaderCircle, Play, Square } from "@lucide/svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import PluginSwitches from "$lib/components/plugins/PluginSwitches.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { pluginDescription, pluginEnabled, pluginName } from "$lib/plugins";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import {
    interpreterPrefs, interpreterStart, interpreterStatus, interpreterStop, saveInterpreter,
    type InterpreterPrefs, type InterpreterStatus,
  } from "./interpreter";

  let { plugin, embedded = false }: PluginPageProps & {
    /** 在右侧栏里（她用 ui.open 打开）：不要大标题。 */
    embedded?: boolean;
  } = $props();

  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  let status = $state<InterpreterStatus | null>(null);
  const targets = $derived(status?.targets ?? []);
  const config = $derived(interpreterPrefs($settingsStore, targets));
  /** 正在输入、还没存的程序名（存好后跟着设置走）。 */
  let appDraft = $state<string | null>(null);
  let saving: Promise<void> = Promise.resolve();
  let busy = $state<"" | "start" | "stop">("");

  // i18n-ignore-start：语言的值和后端 TARGETS 一致（存进设置、写进替你说的话），界面上显示的是右边的文案
  const TARGET_LABELS: Record<string, () => string> = {
    "中文": () => m.plugin_interpreter_target_chinese(),
    "英语": () => m.plugin_interpreter_target_english(),
    "日语": () => m.plugin_interpreter_target_japanese(),
    "韩语": () => m.plugin_interpreter_target_korean(),
  };
  // i18n-ignore-end
  const targetOptions = $derived(targets.map((value) => ({ value, label: TARGET_LABELS[value]?.() ?? value })));
  const displayOptions = $derived([
    { value: "danmaku", label: m.plugin_interpreter_display_danmaku() },
    { value: "bubble", label: m.plugin_interpreter_display_bubble() },
  ]);

  async function refresh() {
    if (!on) {
      status = null;
      return;
    }
    try {
      status = await interpreterStatus();
    } catch (error) {
      toast(false, m.plugin_interpreter_status_failed({ error: errText(error) }));
    }
  }

  onMount(() => {
    const timer = setInterval(() => void refresh(), 2000);
    return () => clearInterval(timer);
  });

  // 打开 / 关掉插件时马上刷新
  $effect(() => {
    if (on) void refresh();
    else status = null;
  });

  function save(patch: Partial<InterpreterPrefs>): Promise<void> {
    saving = saveInterpreter(patch).catch((error) => toast(false, m.shell_toast_save_failed({ error: errText(error) })));
    return saving;
  }

  async function saveApp() {
    if (appDraft === null) return;
    const app = appDraft.trim();
    await save({ app });
    appDraft = null;
  }

  async function run(kind: "start" | "stop", work: () => Promise<string>) {
    busy = kind;
    try {
      // 刚改的设置先存好：开始时后端按设置说那句话
      await saveApp();
      await saving;
      toast(true, m.plugin_interpreter_said({ text: await work() }));
    } catch (error) {
      toast(false, errText(error));
    } finally {
      busy = "";
      void refresh();
    }
  }
</script>

{#snippet body()}
  <div class="space-y-2.5">
    <SettingsSection testid="plugin-interpreter-switch">
      <PluginSwitches {plugin} />
    </SettingsSection>
    {#if !on}
      <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-interpreter-off">{m.plugin_interpreter_off_note()}</p>
    {/if}
  </div>
  {#if on}
    <div class="space-y-2.5">
      <SettingsSection title={m.plugin_interpreter_section_title()} id="interpreter-settings" testid="interpreter-settings">
        <SettingsRow label={m.plugin_interpreter_app_label()} description={m.plugin_interpreter_app_desc()} for="interpreter-app">
          {#snippet control()}
            <div class="w-56 max-w-full">
              <Input id="interpreter-app" data-testid="interpreter-app" class="h-8 text-[13px]" maxlength={64}
                placeholder={m.plugin_interpreter_app_placeholder()}
                bind:value={() => appDraft ?? config.app, (value) => (appDraft = value)}
                onchange={() => void saveApp()}
                onkeydown={(event: KeyboardEvent) => { if (event.key === "Enter") void saveApp(); }} />
            </div>
          {/snippet}
        </SettingsRow>
        <SettingsRow label={m.plugin_interpreter_target_label()}>
          {#snippet control()}
            <div class="w-56 max-w-full">
              <SelectField ariaLabel={m.plugin_interpreter_target_label()} testid="interpreter-target" value={config.target} options={targetOptions}
                onchange={(target) => void save({ target })} />
            </div>
          {/snippet}
        </SettingsRow>
        <SettingsRow label={m.plugin_interpreter_display_label()}>
          {#snippet control()}
            <div class="w-56 max-w-full">
              <SelectField ariaLabel={m.plugin_interpreter_display_label()} testid="interpreter-display" value={config.display} options={displayOptions}
                onchange={(display) => void save({ display: display === "bubble" ? "bubble" : "danmaku" })} />
            </div>
          {/snippet}
        </SettingsRow>
        <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 py-3.5">
          <p class="m-0 flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-foreground" data-testid="interpreter-listening"
            data-listening={status?.listening.length ? "true" : "false"}>
            {#if status?.listening.length}
              <Ear class="size-3.5 shrink-0 text-primary" />{m.plugin_interpreter_listening({ apps: status.listening.join(", ") })}
            {:else}
              <EarOff class="size-3.5 shrink-0 text-muted-foreground" />{m.plugin_interpreter_not_listening()}
            {/if}
          </p>
          <div class="flex shrink-0 items-center gap-2">
            <Button size="sm" data-testid="interpreter-start" disabled={busy !== ""} onclick={() => void run("start", interpreterStart)}>
              {#if busy === "start"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Play class="size-3.5" />{/if}{m.plugin_interpreter_start()}
            </Button>
            <Button size="sm" variant="outline" data-testid="interpreter-stop" disabled={busy !== ""} onclick={() => void run("stop", interpreterStop)}>
              {#if busy === "stop"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Square class="size-3.5" />{/if}{m.plugin_interpreter_stop()}
            </Button>
          </div>
        </div>
      </SettingsSection>
      <p class="m-0 px-1 text-xs leading-relaxed text-muted-foreground" data-testid="interpreter-note">{m.plugin_interpreter_note()}</p>
    </div>
  {/if}
{/snippet}

{#if embedded}
  <div class="space-y-6" data-testid="plugin-page-interpreter">{@render body()}</div>
{:else}
  <SettingsPage title={pluginName(plugin.manifest)} description={pluginDescription(plugin.manifest)} testid="plugin-page-interpreter">
    <div class="space-y-6">{@render body()}</div>
  </SettingsPage>
{/if}
