<script lang="ts">
  import { onMount } from "svelte";
  import { ArrowRight, AudioLines, Captions, Headphones, Languages, Play, SlidersHorizontal, Square } from "@lucide/svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { m } from "$lib/paraglide/messages.js";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { pluginEnabled, setPluginEnabled } from "$lib/plugins";
  import { openTarget } from "$lib/stores/navigation";
  import { runtimeStore } from "$lib/stores/runtime";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { interpreterPrefs, interpreterStart, interpreterStatus, interpreterStop, saveInterpreter, type InterpreterPrefs, type InterpreterStatus } from "./interpreter";

  let { plugin, openSettings, embedded = false }: PluginPageProps & { embedded?: boolean } = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  const runtime = $derived($runtimeStore);
  const needsModel = $derived(!!runtime && !runtime.listenModelId);
  let status = $state<InterpreterStatus | null>(null);
  let statusError = $state("");
  let requestId = 0;
  let appDraft = $state<string | null>(null);
  let saving: Promise<void> = Promise.resolve();
  let busy = $state(false);
  let sent = $state<"start" | "stop" | null>(null);
  let sentAt = 0;
  // i18n-ignore-start：选项的值也是写入插件配置的客户端协议值。
  const labels: Record<string, () => string> = {
    "中文": () => m.plugin_interpreter_target_chinese(), "英语": () => m.plugin_interpreter_target_english(),
    "日语": () => m.plugin_interpreter_target_japanese(), "韩语": () => m.plugin_interpreter_target_korean(),
  };
  // i18n-ignore-end
  const targets = $derived(status?.targets ?? Object.keys(labels));
  const targetOptions = $derived(targets.map((value) => ({ value, label: labels[value]?.() ?? value })));
  const config = $derived(interpreterPrefs($settingsStore, targets));
  const active = $derived(status?.sessionListeners != null);
  const sources = $derived(status?.sessionListeners ?? []);
  const recent = $derived(status?.recent);
  const stopAction = $derived(active || sent === "start");
  const chooseModels = () => openTarget({ page: "advanced", tab: "models", anchor: "models-asr" });

  async function refresh() {
    const id = ++requestId;
    try {
      const next = await interpreterStatus();
      if (id !== requestId) return;
      status = next;
      statusError = "";
      if ((sent === "start" && next?.sessionListeners != null) || (sent === "stop" && next?.sessionListeners == null)
        || Date.now() - sentAt > 15000) sent = null;
    } catch (error) {
      if (id === requestId) statusError = m.plugin_interpreter_status_failed({ error: errText(error) });
    }
  }
  $effect(() => {
    if (on) void refresh();
    else { status = null; sent = null; statusError = ""; appDraft = null; }
    return () => { requestId++; };
  });
  onMount(() => {
    const timer = setInterval(() => { if (on && document.visibilityState === "visible") void refresh(); }, 2000);
    return () => clearInterval(timer);
  });

  // Keep rapid program / language edits ordered. Start reads the configuration only after the last write.
  function save(patch: Partial<InterpreterPrefs>) {
    saving = saving.catch(() => {}).then(() => saveInterpreter(patch));
    return saving;
  }
  async function saveApp() {
    if (appDraft === null) return saving;
    const draft = appDraft;
    await save({ app: draft.trim() });
    if (appDraft === draft) appDraft = null;
  }
  function reportSave(error: unknown) {
    toast(false, m.shell_toast_save_failed({ error: errText(error) }));
  }
  async function run() {
    if (busy) return;
    const stop = stopAction;
    busy = true;
    try {
      if (!stop) await saveApp();
      const text = await (stop ? interpreterStop() : interpreterStart());
      sentAt = Date.now();
      sent = stop ? "stop" : "start";
      toast(true, m.plugin_interpreter_said({ text }));
      await refresh();
    } catch (error) {
      toast(false, errText(error));
    } finally {
      busy = false;
    }
  }
  async function enable() {
    try { await setPluginEnabled(plugin.manifest.id, true); }
    catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
  }
  const clock = (ms: number) => new Date(ms).toLocaleString(undefined, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
</script>

<div class="desk" class:embedded data-testid="plugin-page-interpreter">
  <header>
    <h1><Languages aria-hidden="true" />{m.plugin_interpreter_name()}</h1>
    {#if on}
      <span class="state" class:live={active} data-testid="interpreter-listening" data-listening={sources.length > 0} role="status">
        <span class="lamp"></span>
        {#if statusError}{m.plugin_interpreter_status_unknown()}
        {:else if sent === "stop"}{m.plugin_interpreter_stop_sent()}
        {:else if active && sources.length}{m.plugin_interpreter_listening({ apps: sources.join(", ") })}
        {:else if active}{m.plugin_interpreter_task_active()}
        {:else if sent === "start"}{m.plugin_interpreter_start_sent()}
        {:else if status}{m.plugin_interpreter_not_listening()}
        {:else}{m.plugin_interpreter_reading()}{/if}
      </span>
    {/if}
  </header>
  {#if !on}
    <section class="off" data-testid="plugin-interpreter-off">
      <Headphones aria-hidden="true" /><h2>{m.plugin_interpreter_section_title()}</h2>
      <p>{m.plugin_interpreter_off_note()}</p>
      <button class="action" data-testid="plugin-interpreter-enabled" onclick={enable}>{m.plugin_interpreter_enable()}</button>
    </section>
  {:else}
    <section class="source-bar" aria-label={m.plugin_interpreter_section_title()}>
      <div class="field">
        <label for="interpreter-app">{m.plugin_interpreter_app_label()}</label>
        <div class="process">
          <Headphones aria-hidden="true" />
          <input id="interpreter-app" data-testid="interpreter-app" maxlength="64" placeholder={m.plugin_interpreter_app_placeholder()}
            title={m.plugin_interpreter_app_desc()} disabled={active || busy || !!sent}
            bind:value={() => appDraft ?? config.app, (value) => (appDraft = value)}
            onchange={() => void saveApp().catch(reportSave)} onkeydown={(event) => { if (event.key === "Enter") void saveApp().catch(reportSave); }} />
        </div>
      </div>
      <ArrowRight class="route-arrow" aria-hidden="true" />
      <div class="field">
        <label for="interpreter-target">{m.plugin_interpreter_target_label()}</label>
        <SelectField id="interpreter-target" ariaLabel={m.plugin_interpreter_target_label()} testid="interpreter-target"
          class="target-control" value={config.target} options={targetOptions} disabled={active || busy || !!sent}
          onchange={(target) => void save({ target }).catch(reportSave)} />
      </div>
      {#if needsModel && !stopAction}
        <button class="action" data-testid="interpreter-setup" onclick={chooseModels}><SlidersHorizontal />{m.plugin_interpreter_setup()}</button>
      {:else}
        <button class="action" data-testid={stopAction ? "interpreter-stop" : "interpreter-start"}
          disabled={busy || !status || !!statusError || sent === "stop"} onclick={run}>
          {#if stopAction}<Square />{m.plugin_interpreter_end_session()}{:else}<Play />{m.plugin_interpreter_begin_session()}{/if}
        </button>
      {/if}
    </section>
    {#if statusError}<p class="message" role="alert">{statusError}</p>
    {:else if needsModel && !active}<p class="message">{runtime?.listenError || m.plugin_interpreter_recognizer_missing()}</p>
    {/if}
    <section class="transcript" aria-live="polite" aria-label={m.plugin_interpreter_latest()} data-testid="interpreter-recent">
      <div class="sheet-head">
        <h2><Captions aria-hidden="true" />{m.plugin_interpreter_latest()}</h2>
        {#if recent}<span>{recent.app ?? ""} <time datetime={new Date(recent.atMs).toISOString()}>{clock(recent.atMs)}</time></span>{/if}
      </div>
      {#if recent}
        <div class="pair">
          {#if recent.heard}
            <div class="original">
              <span class="caption">{m.plugin_interpreter_original()}</span>
              <p data-testid="interpreter-original">{recent.heard}</p>
            </div>
          {/if}
          <div class="translation">
            <span class="caption">{m.plugin_interpreter_translation()}</span>
            <p data-testid="interpreter-translation">{recent.text}</p>
          </div>
        </div>
      {:else}
        <div class="empty">
          <AudioLines aria-hidden="true" />
          <p>{active ? m.plugin_interpreter_waiting_line() : m.plugin_interpreter_no_line()}</p>
        </div>
      {/if}
    </section>
    <footer>
      <span class="output"><span class="caption">{m.plugin_interpreter_display_label()}</span>{config.display === "bubble" ? m.plugin_interpreter_display_bubble() : m.plugin_interpreter_display_danmaku()}</span>
      <button class="options" data-testid="interpreter-options" onclick={() => openSettings ? openSettings() : openTarget({ page: "plugin", id: plugin.manifest.id, params: { view: "settings" } })}>
        <SlidersHorizontal aria-hidden="true" />{m.plugin_interpreter_options()}
      </button>
    </footer>
    <p class="note" data-testid="interpreter-note">{runtime?.listenOnline ? m.plugin_interpreter_online_cost() : m.plugin_interpreter_translation_cost()}</p>
  {/if}
</div>

<style>
  .desk { --signal: light-dark(#096c60, #88d9bf); --button: light-dark(#146956, #a3e6cd); --button-ink: light-dark(#fff, #102a24); --rule: color-mix(in srgb, var(--foreground) 14%, transparent);
    container-type: inline-size; box-sizing: border-box; width: 100%; max-width: 980px; margin: 0 auto; padding: clamp(20px,4.7vw,58px) clamp(18px,5.2vw,62px); color: var(--foreground); }
  .desk.embedded { padding: 10px 4px 18px; }
  header, .sheet-head, footer { display:flex; align-items:center; justify-content:space-between; gap:12px; }
  header { flex-wrap:wrap; padding-bottom: 24px; border-bottom: 1px solid var(--rule); }
  h1, h2, p { margin:0; }
  h1 { display:flex; align-items:center; gap:13px; font-size:clamp(25px,4.2cqw,38px); line-height:1.17; font-weight:680; letter-spacing:-.045em; }
  h1 :global(svg) { width:1.04em; height:1.04em; color:var(--signal); stroke-width:1.6; }
  .state { display:inline-flex; align-items:center; gap:7px; color:var(--muted-foreground); font-size:12px; overflow-wrap:anywhere; }
  .lamp { width:7px; height:7px; flex:none; border:1px solid currentColor; border-radius:50%; }
  .state.live { color:var(--signal); }
  .state.live .lamp { background:currentColor; }
  .source-bar { display:grid; grid-template-columns:minmax(135px,1.4fr) auto minmax(115px,.9fr) auto; gap:14px; align-items:end; padding:24px 0; }
  .field { display:grid; gap:8px; min-width:0; }
  .field label, .caption { color:var(--muted-foreground); font-size:12px; font-weight:550; }
  .process, .field :global(.target-control) { height:43px; min-width:0; background:var(--card); color:var(--foreground); border:1px solid var(--rule); border-radius:7px; }
  .process { display:flex; align-items:center; padding:0 11px; gap:9px; }
  .process:focus-within { outline:2px solid var(--signal); outline-offset:2px; }
  .process :global(svg), .desk :global(.route-arrow) { width:16px; height:16px; color:var(--muted-foreground); flex:none; }
  .desk :global(.route-arrow) { margin-bottom:14px; }
  input { width:100%; min-width:0; font-size:14px; background:none; color:inherit; border:none; outline:none; }
  input::placeholder { color:var(--foreground); }
  input:disabled, .field :global(.target-control:disabled) { opacity:.7; }
  .field :global(.target-control) { font-size:14px; }
  .action { display:inline-flex; justify-content:center; align-items:center; gap:8px; min-height:43px; padding:0 17px; background:var(--button); color:var(--button-ink); font-size:14px; font-weight:650; border-radius:7px; cursor:pointer; }
  .action:disabled { opacity:.5; cursor:default; }
  .action:hover:enabled { filter:brightness(.94); }
  .action :global(svg), .options :global(svg) { width:15px; height:15px; }
  .message { color:var(--muted-foreground); font-size:12px; margin:-12px 0 20px; }
  .transcript { border-block:1px solid var(--rule); margin-top:4px; padding:19px 0 22px; }
  .sheet-head { flex-wrap:wrap; }
  .sheet-head h2 { display:flex; align-items:center; gap:8px; font-size:14px; font-weight:650; }
  .sheet-head h2 :global(svg) { width:18px; height:18px; color:var(--signal); }
  .sheet-head > span { color:var(--muted-foreground); font-size:11px; font-variant-numeric:tabular-nums; overflow-wrap:anywhere; }
  time { white-space:nowrap; margin-left:6px; }
  .pair { display:grid; gap:20px; margin-top:26px; padding:0 0 4px; }
  .pair p { overflow-wrap:anywhere; margin-top:5px; }
  .original p { font-size:clamp(16px,2.4cqw,20px); line-height:1.5; }
  .translation { border-left:3px solid var(--signal); padding-left:14px; }
  .translation .caption { color:var(--signal); }
  .translation p { font-size:clamp(22px,3.8cqw,32px); line-height:1.45; letter-spacing:-.025em; font-weight:600; }
  .empty { display:flex; flex-direction:column; align-items:center; justify-content:center; gap:14px; min-height:162px; color:var(--muted-foreground); font-size:13px; text-align:center; }
  .empty :global(svg) { width:32px; height:32px; stroke-width:1.25; color:var(--signal); }
  footer { padding:17px 0 0; flex-wrap:wrap; }
  .output, .options { display:inline-flex; align-items:center; gap:9px; font-size:12px; }
  .options { color:var(--muted-foreground); cursor:pointer; }
  .options:hover { color:var(--foreground); }
  .note { margin-top:9px; color:var(--muted-foreground); font-size:12px; line-height:1.6; }
  .off { display:grid; justify-items:start; gap:14px; max-width:480px; padding:32px 0; }
  .off > :global(svg) { width:28px; height:28px; color:var(--signal); }
  .off h2 { font-size:20px; font-weight:650; }
  .off p { color:var(--muted-foreground); line-height:1.65; font-size:14px; }
  @container (max-width: 540px) {
    .source-bar { grid-template-columns:minmax(0,1.25fr) auto minmax(0,1fr); gap:10px; }
    .source-bar > .action { grid-column:1/-1; margin-top:4px; }
    header { padding-bottom:18px; }
  }
  @container (max-width: 260px) {
    .source-bar { grid-template-columns:1fr; }
    .desk :global(.route-arrow) { display:none; }
  }
</style>
