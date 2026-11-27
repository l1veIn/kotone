<script lang="ts">
  import { ArrowRight, AudioLines, Check, ChevronRight, Download, Headphones, LoaderCircle, Mic, Play, Power, SlidersHorizontal, Volume2 } from "@lucide/svelte";
  import { Switch } from "$lib/components/ui/switch";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { pluginEnabled, setPluginEnabled } from "$lib/plugins";
  import { companionName } from "$lib/stores/companion";
  import { openTarget } from "$lib/stores/navigation";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import { PLUGIN_ID, voiceChatInstall, voiceChatSay, voiceChatStatus, voiceChatTest, type VoiceChatStatus } from "./voiceChat";

  let { plugin, openSettings, embedded = false }: PluginPageProps & { embedded?: boolean } = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  let status = $state<VoiceChatStatus | null>(null);
  let loadError = $state("");
  let busy = $state<"" | "switch" | "install" | "test" | "say">("");
  let result = $state<Awaited<ReturnType<typeof voiceChatTest>> | null>(null);
  let installerNotice = $state("");
  let requestId = 0;
  const running = $derived(!!(on && status?.enabled && status.driverInstalled && status.running && !status.error));
  const db = $derived(result && result.peak > 0 ? 20 * Math.log10(result.peak) : -Infinity);
  const fill = $derived(Math.min(100, Math.max(0, (db + 60) / 60 * 100)));

  async function refresh() {
    const request = ++requestId;
    try {
      const next = await voiceChatStatus();
      if (request !== requestId) return;
      if (next?.activeMic !== status?.activeMic || next?.running !== status?.running || next?.error || !next?.enabled) result = null;
      status = next;
      loadError = next ? "" : m.plugin_game_voice_unavailable();
    } catch (error) {
      if (request !== requestId) return;
      status = null;
      result = null;
      loadError = m.plugin_game_voice_status_failed({ error: errText(error) });
    }
  }

  $effect(() => {
    if (!on) {
      status = null;
      result = null;
      loadError = "";
      installerNotice = "";
      return;
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => { requestId++; clearInterval(timer); };
  });

  async function run(kind: Exclude<typeof busy, "">, task: () => Promise<void>) {
    if (busy) return;
    busy = kind;
    try { await task(); }
    catch (error) { toast(false, errText(error)); }
    finally {
      busy = "";
      if (on) void refresh();
    }
  }
  const toggle = (enabled: boolean) => run("switch", () => setPluginEnabled(PLUGIN_ID, enabled));
  const install = () => run("install", async () => {
    const response = await voiceChatInstall();
    installerNotice = response.installed ? "" : (response.note ?? m.plugin_game_voice_installer_ran());
    toast(response.installed, response.installed ? m.plugin_game_voice_installed() : installerNotice);
  });
  const test = () => run("test", async () => {
    result = null;
    const measured = await voiceChatTest();
    if (on) result = measured;
  });
  const say = () => run("say", async () => {
    await voiceChatSay(m.plugin_game_voice_test_phrase());
    toast(true, m.plugin_game_voice_said());
  });
  const devices = () => openSettings ? openSettings() : openTarget({ page: "plugin", id: PLUGIN_ID, params: { view: "settings" } });

  const stateText = $derived.by(() => {
    if (!on) return m.plugin_game_voice_off();
    if (loadError) return m.plugin_game_voice_unavailable();
    if (!status) return m.plugin_game_voice_state_loading();
    if (!status.enabled) return m.plugin_game_voice_companion_off({ name: $companionName });
    if (status.installing || busy === "install") return m.plugin_game_voice_state_installing();
    if (!status.driverInstalled) return m.plugin_game_voice_state_not_installed();
    if (status.error) return m.plugin_game_voice_unavailable();
    return running ? m.plugin_game_voice_routing() : m.plugin_game_voice_state_starting();
  });
  const description = $derived.by(() => {
    if (loadError) return loadError;
    if (!on) return m.plugin_game_voice_intro({ name: $companionName });
    if (status && !status.enabled) return m.plugin_game_voice_companion_needed({ name: $companionName });
    if (status && !status.driverInstalled) return installerNotice || m.plugin_game_voice_install_hint();
    if (status?.error) return status.error;
    return m.plugin_game_voice_tone_hint();
  });
</script>

<div class="voice-page ui-scope" class:embedded data-testid="plugin-page-game-voice">
  <div class="desk">
    <section class="console" aria-label={m.plugin_game_voice_desk_title()}>
      <div class="console-top">
        <div class="signal-status" class:routing={running} data-testid="voice-chat-state" role="status">
          <span class="lamp" aria-hidden="true"></span>{stateText}
        </div>
        {#if on}
          <Switch data-testid="plugin-game-voice-enabled" aria-label={m.system_plugins_enable({ name: m.plugin_game_voice_name() })}
            checked={on} disabled={busy !== ""} onCheckedChange={(value) => void toggle(value)} />
        {/if}
      </div>

      <div class="panel">
        <div class="controls">
          <h1>{m.plugin_game_voice_desk_title()}</h1>
          <p class="description" data-testid={!on ? "plugin-game-voice-off" : undefined}>{description}</p>
          <div class="buttons">
            {#if !on}
              <button class="key primary" role="switch" aria-checked={false} type="button" data-testid="plugin-game-voice-enabled"
                disabled={busy !== ""} onclick={() => void toggle(true)}>
                <Power size={16} />{m.plugin_game_voice_enable()}
              </button>
            {:else if status && !status.enabled}
              <button class="key primary" type="button" onclick={() => openTarget({ page: "setup-key" })}>{m.plugin_game_voice_open_setup({ name: $companionName })}</button>
            {:else if status?.enabled && !status.driverInstalled}
              <button class="key primary" type="button" data-testid="voice-chat-install" disabled={busy !== "" || status.installing} onclick={() => void install()}>
                {#if busy === "install" || status.installing}<LoaderCircle size={16} class="animate-spin" />{:else}<Download size={16} />{/if}
                {m.plugin_game_voice_install()}
              </button>
            {:else if loadError}
              <button class="key" type="button" onclick={() => void refresh()}>{m.plugin_game_voice_retry()}</button>
            {:else if status?.enabled && status.error}
              <button class="key" type="button" onclick={devices}><SlidersHorizontal size={16} />{m.plugin_game_voice_devices()}</button>
            {:else if status?.enabled}
              <button class="key primary" type="button" data-testid="voice-chat-test" disabled={busy !== "" || !running} onclick={() => void test()}>
                {#if busy === "test"}<LoaderCircle size={16} class="animate-spin" />{:else}<Play size={15} fill="currentColor" />{/if}
                {m.plugin_game_voice_test_tone()}
              </button>
              <button class="key" type="button" data-testid="voice-chat-say" title={m.plugin_game_voice_test_phrase()}
                disabled={busy !== "" || !running || !status.ttsCanSpeak} onclick={() => void say()}>
                <Volume2 size={16} />{m.plugin_game_voice_say()}
              </button>
            {/if}
          </div>
        </div>
        <div class="meter" data-testid="voice-chat-meter">
          <div class="meter-label"><AudioLines size={17} />{m.plugin_game_voice_peak()}</div>
          <div class="reading" aria-hidden="true">{result ? Number.isFinite(db) ? db.toFixed(1) : "−∞" : "—"} <span>dBFS</span></div>
          <div class="bar" aria-hidden="true"><i style:width={fill + "%"}></i></div>
          <div class="meter-note" role="status" data-testid="voice-chat-test-result">
            {#if result?.ok}<Check size={13} />{/if}
            {busy === "test" ? m.plugin_game_voice_measuring() : result ? result.ok ? m.plugin_game_voice_measured_ok() : m.plugin_game_voice_measured_fail() : m.plugin_game_voice_not_measured()}
            {#if result}<span class="sr-only">{Number.isFinite(db) ? db.toFixed(1) : "−∞"} dBFS</span>{/if}
          </div>
        </div>
      </div>
      {#if on && status && !status.driverInstalled && status.attribution}
        <p class="console-note">{status.attribution}</p>
      {:else if on && running && !status?.ttsCanSpeak}
        <p class="console-note">{m.plugin_game_voice_try_desc_system({ name: $companionName })}</p>
      {:else if on && result && !result.ok}
        <p class="console-note">{result.note}</p>
      {/if}
    </section>

    {#if on && status?.driverInstalled}
      <div class="route">
        <div class="port"><Mic size={19} /><div><span>{m.plugin_game_voice_mic_label()}</span><strong data-testid="voice-chat-active-mic">{status.activeMic ?? (status.includeMic ? m.plugin_game_voice_disconnected() : m.plugin_game_voice_mic_excluded())}</strong></div></div>
        <ArrowRight size={16} class="route-arrow" />
        <div class="port"><Headphones size={19} /><div><span>{m.plugin_game_voice_game_mic()}</span><strong>{status.devices.cableOutput ?? m.plugin_game_voice_disconnected()}</strong></div></div>
      </div>
    {/if}
    {#if on}
      <button type="button" class="settings-link" data-testid="voice-chat-open-settings" onclick={devices}>
        <SlidersHorizontal size={16} />{m.plugin_game_voice_devices()}<ChevronRight size={15}/>
      </button>
    {/if}
  </div>
</div>

<style>
  .voice-page { container-type: inline-size; color: var(--foreground); }
  .desk { max-width: 1000px; margin: 0 auto; padding: clamp(22px, 4.5cqw, 48px); }
  .embedded .desk { padding: 8px 0; }
  .console { background: #182220; color: #eef5ec; border: 1px solid #384940; border-top: 3px solid #9fc79d; border-radius: 5px; padding: clamp(19px, 3.6cqw, 36px); }
  .console-top, .signal-status, .buttons, .key, .port, .settings-link, .meter-label, .meter-note { display: flex; align-items: center; }
  .console-top { justify-content: space-between; gap: 14px; min-height: 22px; }
  .signal-status { gap: 8px; color: #c9d5cc; font-size: 12px; }
  .lamp { flex: none; height: 7px; width: 7px; border-radius: 100%; background: #939f97; }
  .routing .lamp { background: #b5ea94; }
  .panel { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(150px, .85fr); align-items: end; gap: clamp(22px, 4cqw, 48px); margin-top: clamp(22px, 4cqw, 42px); }
  h1 { margin: 0; font-size: clamp(28px, 4.5cqw, 46px); line-height: 1.16; font-weight: 650; letter-spacing: -.045em; }
  .description { color: #c4d1c8; margin: 12px 0 0; font-size: 14px; line-height: 1.7; overflow-wrap: anywhere; }
  .buttons { flex-wrap: wrap; gap: 10px; margin-top: 22px; }
  .buttons:empty { display: none; }
  .key { gap: 8px; min-height: 37px; padding: 8px 16px; border: 1px solid #526459; border-radius: 4px; background: transparent; color: #ecf1e9; font-size: 13px; font-weight: 600; cursor: pointer; }
  .key.primary { background: #cee7aa; color: #1b291a; border-color: #cee7aa; }
  .key:hover:enabled { border-color: #b9cdba; background: #28372f; }
  .key.primary:hover:enabled { background: #deefc6; }
  .key:disabled { opacity: .48; cursor: default; }
  .key:focus-visible { outline: 2px solid #f6ffe9; outline-offset: 3px; }
  .meter { min-width: 0; padding: 13px 15px 12px; border: 1px solid #435448; border-radius: 3px; background: #111a16; }
  .meter-label, .meter-note { gap: 6px; color: #c1cfc3; font-size: 11px; }
  .reading { margin-top: 10px; color: #d4ecb6; font: 500 32px/1.1 "Consolas", monospace; letter-spacing: -.07em; }
  .reading span { color: #a8bba9; font-size: 12px; letter-spacing: 0; }
  .bar { position: relative; height: 8px; margin: 13px 0 7px; background: #3c4f41; }
  .bar i { display: block; height: 100%; background: #c2e5a1; }
  .bar::after { content: ""; position: absolute; inset: 0; background: repeating-linear-gradient(90deg, transparent 0 calc(5% - 2px), #111a16 calc(5% - 2px) 5%); }
  .console-note { color: #c4d1c8; margin: 19px 0 0; border-top: 1px solid #384940; padding-top: 12px; font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
  .route { display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; column-gap: 20px; padding: 23px 3px; border-bottom: 1px solid var(--hairline); }
  .port { gap: 12px; min-width: 0; }
  .port :global(svg), .route :global(.route-arrow) { color: var(--muted-foreground); flex: none; }
  .port span, .port strong { display: block; }
  .port span { color: var(--muted-foreground); font-size: 12px; }
  .port strong { margin-top: 3px; font-size: 14px; font-weight: 600; overflow-wrap: anywhere; }
  .settings-link { gap: 7px; padding: 10px 3px; margin-top: 8px; color: var(--muted-foreground); font-size: 13px; cursor: pointer; }
  .settings-link:hover { color: var(--foreground); }
  @container (max-width: 580px) {
    .panel { grid-template-columns: minmax(0, 1fr); }
    .meter { max-width: none; }
    .reading { font-size: 24px; }
    .route { grid-template-columns: 1fr; gap: 12px; padding: 18px 0; }
    .route :global(.route-arrow) { display: none; }
  }
</style>
