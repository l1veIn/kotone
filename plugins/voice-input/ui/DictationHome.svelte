<script lang="ts">
  import { onMount } from "svelte";
  import { ArrowUpRight, AudioLines, Keyboard, Mic, Square } from "@lucide/svelte";
  import { hotkeyLabel } from "$lib/hotkeyCombo";
  import type { AudioDevice, HotkeyStatus, KeyUse } from "$lib/ipc";
  import { m } from "$lib/paraglide/messages.js";
  import { openTarget } from "$lib/stores/navigation";
  import { asrStore } from "$lib/stores/runtime";
  import { errText, settingsStore } from "$lib/stores/ui";
  import { beginVoiceInputTrial, cancelVoiceInputTrial, finishVoiceInputTrial } from "$lib/voiceInputTrial";
  import AsrCard from "./AsrCard.svelte";

  let { devices, hotkeyStatus, talkBinding, onOpenSound, onOpenModels }: {
    devices: AudioDevice[] | null;
    hotkeyStatus: HotkeyStatus | null;
    talkBinding: KeyUse | null;
    onOpenSound: () => void;
    onOpenModels: () => void;
  } = $props();

  // 热键表已把别名、修饰键顺序规范化。老设置中未识别的值整串保留，供用户去改。
  const shortcut = $derived(talkBinding?.key.trim() ?? $settingsStore?.hotkey.key.trim() ?? "");
  const keys = $derived(talkBinding?.combo ? talkBinding.combo.split("+") : shortcut ? [shortcut] : []);
  const mode = $derived($settingsStore?.interactionMode);
  const hint = $derived(mode === "one-shot" ? m.plugin_voice_input_key_once()
    : mode === "solo" ? m.voice_mode_solo_desc()
    : mode === "dictation" || (!mode && $settingsStore?.hotkey.mode === "toggle") ? m.voice_mode_dictation_desc()
    : m.plugin_voice_input_key_hold());
  const mic = $derived(devices?.find((d) => d.id === ($settingsStore?.audioDeviceId || "default")));
  const asrReady = $derived($asrStore === "local-running" || $asrStore === "online-ready");
  const loading = $derived(devices === null || $asrStore === "unknown");
  const preparing = $derived($asrStore === "local-starting" || $asrStore === "online-preparing");

  let pane: HTMLElement;
  let phase = $state<"idle" | "starting" | "recording" | "recognizing">("idle");
  let session: string | null = null;
  let revision = 0;
  let stopTimer: ReturnType<typeof setTimeout> | undefined;
  let transcript = $state("");
  let message = $state("");
  let failure = $state(false);

  async function start() {
    if (phase !== "idle" || !mic || !asrReady) return;
    const current = ++revision;
    phase = "starting";
    message = "";
    transcript = "";
    failure = false;
    try {
      const id = await beginVoiceInputTrial();
      if (current !== revision) { await cancelVoiceInputTrial(id); return; }
      session = id;
      phase = "recording";
      stopTimer = setTimeout(() => void finish(), 30_000);
    } catch (error) {
      if (current === revision) { phase = "idle"; message = errText(error); failure = true; }
    }
  }
  async function finish() {
    if (phase !== "recording" || session === null) return;
    clearTimeout(stopTimer);
    const current = revision;
    phase = "recognizing";
    try {
      const text = (await finishVoiceInputTrial(session)).trim();
      if (current !== revision) return;
      transcript = text;
      if (!text) message = m.plugin_voice_input_try_empty();
    } catch (error) {
      if (current === revision) { message = errText(error); failure = true; }
    } finally {
      if (current === revision) { session = null; phase = "idle"; }
    }
  }
  function cancel() {
    if (phase === "idle") return;
    clearTimeout(stopTimer);
    ++revision;
    const id = session;
    session = null;
    phase = "idle";
    if (id !== null) void cancelVoiceInputTrial(id).catch(() => {});
  }
  onMount(() => {
    // 宿主打开插件设置时保留 App 挂载，仅把它隐藏。与切走分页、关闭窗口同样释放麦克风。
    const hidden = () => { if (document.hidden || !pane?.getClientRects().length) cancel(); };
    const resized = new ResizeObserver(hidden);
    resized.observe(pane);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      resized.disconnect();
      document.removeEventListener("visibilitychange", hidden);
      cancel();
    };
  });
</script>

<section class="home" bind:this={pane} data-testid="voice-dictation-home" aria-label={m.voice_shell_title()}>
  <header class="shortcut">
    <div class="key-caption"><Keyboard size={16} />{m.voice_hotkey_name_record()}</div>
    <div class="key-line" data-testid="voice-home-shortcut">
      {#if shortcut}
        {#each keys as key, i}
          {#if i > 0}<span class="plus" aria-hidden="true">+</span>{/if}
          <kbd>{talkBinding?.combo ? hotkeyLabel(key) : key}</kbd>
        {/each}
      {:else}
        <span class="unset">{m.voice_hotkey_unset()}</span>
      {/if}
    </div>
    {#if shortcut && (!talkBinding || talkBinding.combo) && !hotkeyStatus?.error && (!asrReady || hotkeyStatus?.registered !== false)}
      <p class="mode" data-testid="voice-home-key-mode">{hint}</p>
    {:else}
      <p class="mode problem">{hotkeyStatus?.error || (shortcut ? m.plugin_voice_input_key_inactive() : m.plugin_voice_input_key_needed())}
        <button class="link" onclick={() => openTarget({ page: "advanced", tab: "hotkeys", anchor: "hotkey-voice-input.talk" })}>{m.hotkeys_change_in_table()}<ArrowUpRight size={13} /></button>
      </p>
    {/if}
  </header>

  <section class="trial" data-testid="voice-trial" data-phase={phase} aria-labelledby="trial-title">
    <div class="trial-top">
      <h2 id="trial-title"><AudioLines size={18} />{m.plugin_voice_input_try_title()}</h2>
      <div class="controls">
        {#if phase !== "idle"}
          <button type="button" class="link" onclick={cancel} data-testid="voice-trial-cancel">{m.voice_action_cancel()}</button>
        {/if}
        {#if phase === "recording"}
          <button type="button" class="primary" onclick={() => void finish()} data-testid="voice-trial-finish"><Square size={13}/>{m.plugin_voice_input_try_finish()}</button>
        {:else if phase === "idle" && !loading && !mic}
          <button type="button" class="primary" onclick={onOpenSound} data-testid="voice-trial-setup">{m.plugin_voice_input_pick_mic()}<ArrowUpRight size={14}/></button>
        {:else if phase === "idle" && !loading && !asrReady}
          <button type="button" class="primary" onclick={onOpenModels} data-testid="voice-trial-setup">{m.plugin_voice_input_setup_asr()}<ArrowUpRight size={14}/></button>
        {:else}
          <button type="button" class="primary" disabled={phase !== "idle" || loading} onclick={() => void start()} data-testid="voice-trial-start">
            <Mic size={14}/>{phase === "recognizing" ? m.plugin_voice_input_try_working() : phase === "starting" ? m.plugin_voice_input_try_starting() : m.plugin_voice_input_try_start()}
          </button>
        {/if}
      </div>
    </div>
    <div class="paper" class:spoken={!!transcript} class:problem={failure} role="status" aria-live="polite" data-testid="voice-trial-result">
      {#if transcript}{transcript}
      {:else if message}{message}
      {:else if phase === "recording"}<span class="record-dot"></span>{m.plugin_voice_input_try_listening()}
      {:else if phase === "recognizing"}{m.plugin_voice_input_try_working()}
      {:else if loading || phase === "starting"}{m.plugin_voice_input_checking()}
      {:else if !mic}{m.plugin_voice_input_mic_missing()}
      {:else if !asrReady}{preparing ? m.plugin_voice_input_asr_preparing() : m.plugin_voice_input_try_asr_needed()}
      {:else}{m.plugin_voice_input_try_prompt()}{/if}
    </div>
    <p class="footnote">{m.plugin_voice_input_try_scope()}</p>
  </section>

  <div class="sources">
    <div class="microphone">
      <h2>{m.plugin_voice_input_microphone()}</h2>
      <p data-testid="voice-home-mic">{mic?.name ?? (devices === null ? m.plugin_voice_input_checking() : m.plugin_voice_input_mic_missing())}</p>
      <button type="button" class="link" onclick={onOpenSound}>{m.plugin_voice_input_mic_settings()}<ArrowUpRight size={13}/></button>
    </div>
    <AsrCard {onOpenModels} />
  </div>
</section>

<style>
  .home { --ink: var(--color-foreground); --muted: var(--color-muted-foreground); --line: var(--color-hairline); --accent: var(--color-primary); color: var(--ink); container-type: inline-size; }
  .shortcut { padding: 6px 0 22px; }
  .key-caption, .link, .trial h2, .controls, .primary, .key-line { display: flex; align-items: center; }
  .key-caption { color: var(--muted); gap: 8px; font-size: 12px; }
  .key-line { flex-wrap: wrap; gap: 8px; margin: 13px 0 12px; }
  kbd { max-width: 100%; overflow-wrap: anywhere; border: 1px solid color-mix(in srgb, var(--ink) 21%, transparent); border-bottom-width: 4px; border-radius: 8px; background: var(--color-card); padding: 4px 13px 5px; font: 600 clamp(23px, 5cqi, 37px)/1.15 ui-monospace, Consolas, monospace; letter-spacing: -.045em; }
  .plus { color: var(--muted); font-size: 19px; }
  .unset { font-size: 24px; }
  .mode { margin: 0; font-size: 15px; }
  .mode .link { display: inline-flex; margin-inline-start: 6px; }
  .problem { color: var(--color-destructive); overflow-wrap: anywhere; }
  .trial { border-block: 1px solid var(--line); background-color: color-mix(in srgb, var(--accent) 3%, transparent); padding: 18px; }
  .trial-top { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; }
  .trial h2 { gap: 8px; font-size: 15px; font-weight: 650; margin: 0; }
  .trial h2 :global(svg) { color: var(--accent); }
  .controls { gap: 12px; flex-wrap: wrap; }
  .primary { justify-content: center; gap: 6px; min-height: 33px; padding: 5px 12px; border-radius: 6px; background: var(--accent); color: var(--color-primary-foreground); font-size: 13px; font-weight: 600; }
  .primary:disabled { opacity: .55; }
  .link { gap: 4px; color: var(--accent); font-size: 12px; }
  .link:hover { text-decoration: underline; }
  .paper { min-height: 75px; padding: 15px 0 12px; color: var(--muted); font-size: 15px; line-height: 1.65; white-space: pre-wrap; overflow-wrap: anywhere; }
  .paper.spoken { color: var(--ink); font-size: 20px; }
  .paper.problem { color: var(--color-destructive); }
  .footnote { margin: 0; font-size: 12px; color: var(--muted); }
  .record-dot { display: inline-block; width: 8px; height: 8px; margin-inline-end: 8px; border-radius: 50%; background: var(--color-destructive); }
  .sources { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 22px; margin-top: 23px; }
  .microphone h2 { margin: 0 0 5px; color: var(--muted); font-size: 12px; font-weight: normal; }
  .microphone p { margin: 0 0 5px; overflow-wrap: anywhere; font-size: 13px; font-weight: 500; }
  @container (max-width: 460px) {
    .sources { grid-template-columns: minmax(0, 1fr); gap: 14px; }
    .trial { padding-inline: 12px; }
  }
</style>
