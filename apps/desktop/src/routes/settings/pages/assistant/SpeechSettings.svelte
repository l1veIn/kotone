<script lang="ts">
  /*
   * 她的声音（ADR 048）：只在这里配。选服务商 → 就地填 Key（先试一次再存，存进 Windows 的凭据库）→ 选声音（能试听）。
   * Key 一家一份：声音设置里没填就用同一家 API 连接的 Key；页面上写明 Key 从哪来、怎么计费。“自动”跟着她用的模型走。
   */
  import { onMount } from "svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { isTauri, openExternal, type AssistantConfig } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import {
    cancelSpeechModelDownload, clearSpeechKey, downloadSpeechModel, getSpeechCurrent, getSpeechModelStatus,
    listSpeechProviders, listSpeechVoices, saveSpeechKey, subscribeSpeechModel, tryVoice,
    type SpeechCheck, type SpeechCurrent, type SpeechModelStatus, type SpeechProvider, type SpeechVoice,
  } from "../../../../lib/assistant";

  let model = $state<SpeechModelStatus | null>(null);
  let voices = $state<SpeechVoice[]>([]);
  let providers = $state<SpeechProvider[]>([]);
  let current = $state<SpeechCurrent | null>(null);
  let error = $state("");
  let voiceError = $state("");
  let saving = $state(false);
  let starting = $state(false);
  let cancelling = $state(false);
  let keyInput = $state("");
  let keyOpen = $state(false);
  let checking = $state(false);
  let checked = $state<SpeechCheck | null>(null);
  let trying = $state(false);
  let disposed = false;
  let modelRevision = 0;
  const config = $derived($settingsStore?.assistant);
  const phases: Record<SpeechModelStatus["phase"], string> = {
    idle: m.companion_speech_phase_idle(), downloading: m.companion_speech_phase_downloading(),
    verifying: m.companion_speech_phase_verifying(), extracting: m.companion_speech_phase_extracting(),
    installed: m.companion_speech_phase_installed(), error: m.companion_speech_phase_error(),
    cancelled: m.companion_speech_phase_cancelled(),
  };

  /** 在线的几家：清单 ID ↔ 声音编号的前缀（和后端 `kotone_tts::catalog::PROVIDERS` 同一个顺序）。 */
  const ONLINE = ["deepseek-tts", "minimax-tts", "dashscope-tts", "siliconflow-tts", "volcano-tts", "openai-compat-tts"] as const;
  type OnlineId = (typeof ONLINE)[number];
  type Engine = "auto" | "system" | "kokoro" | OnlineId;
  const PREFIX: Record<OnlineId, string> = {
    "deepseek-tts": "deepseek", "minimax-tts": "minimax", "dashscope-tts": "dashscope",
    "siliconflow-tts": "siliconflow", "volcano-tts": "volcano", "openai-compat-tts": "openai",
  };
  const isOnline = (id: string): id is OnlineId => (ONLINE as readonly string[]).includes(id);
  const onlineOfPrefix = (prefix: string): OnlineId | undefined => ONLINE.find((id) => PREFIX[id] === prefix);
  const NAMES: Record<OnlineId, () => string> = {
    "deepseek-tts": m.companion_speech_name_deepseek, "minimax-tts": m.companion_speech_name_minimax,
    "dashscope-tts": m.companion_speech_name_dashscope, "siliconflow-tts": m.companion_speech_name_siliconflow,
    "volcano-tts": m.companion_speech_name_volcano, "openai-compat-tts": m.companion_speech_name_openai,
  };
  const BILLING: Record<OnlineId, () => string> = {
    "deepseek-tts": m.companion_speech_billing_deepseek, "minimax-tts": m.companion_speech_billing_minimax,
    "dashscope-tts": m.companion_speech_billing_dashscope, "siliconflow-tts": m.companion_speech_billing_siliconflow,
    "volcano-tts": m.companion_speech_billing_volcano, "openai-compat-tts": m.companion_speech_billing_openai,
  };
  const LABELS: Record<Engine, () => string> = {
    auto: m.companion_speech_engine_auto, system: m.companion_speech_engine_system, kokoro: m.companion_speech_engine_kokoro,
    "deepseek-tts": m.companion_speech_engine_deepseek, "minimax-tts": m.companion_speech_engine_minimax,
    "dashscope-tts": m.companion_speech_engine_dashscope, "siliconflow-tts": m.companion_speech_engine_siliconflow,
    "volcano-tts": m.companion_speech_engine_volcano, "openai-compat-tts": m.companion_speech_engine_openai,
  };
  /** 内置的默认声音（后端的服务商表没读到时用）。 */
  const DEFAULT_VOICE: Record<OnlineId, string> = {
    "deepseek-tts": "stella", "minimax-tts": "female-shaonv", "dashscope-tts": "Cherry",
    "siliconflow-tts": "FunAudioLLM/CosyVoice2-0.5B:claire", "volcano-tts": "BV700_streaming", "openai-compat-tts": "",
  };

  /** 声音编号统一写成「提供方/声音」；旧配置里不带前缀的值照常匹配。 */
  function qualifiedVoice(prefix: string, id: string): string {
    const value = id.trim();
    if (!value) return "";
    if (value.startsWith(`${prefix}/`)) return value;
    if (prefix === "kokoro" && value.startsWith("kokoro:")) return `kokoro/${value.slice(7)}`;
    return `${prefix}/${value}`;
  }
  /**
   * 选中的是哪个：在线的记在新字段 ttsModelId，本地的在旧字段 speechEngine。
   * ttsModelId 为空且本地是系统语音 = “自动”（默认）；明确选了系统语音写 ttsModelId = "local"。
   */
  const engine = $derived<Engine | undefined>(
    !config ? undefined
      : config.ttsModelId && isOnline(config.ttsModelId) ? config.ttsModelId
      : config.speechEngine === "kokoro" ? "kokoro"
      : config.ttsModelId === "local" ? "system"
      : "auto",
  );
  /** 实际在用的那一家（“自动”时看后端选的）。 */
  const actual = $derived<string>(engine === "auto" ? (current?.modelId ?? "system-speech") : engine ?? "system-speech");
  const actualOnline = $derived<OnlineId | undefined>(isOnline(actual) ? actual : undefined);
  /** 声音属于哪个提供方（声音编号的前缀）。 */
  const voicePrefix = $derived(actualOnline ? PREFIX[actualOnline] : actual === "kokoro" || actual === "kokoro-zh" ? "kokoro" : "system");
  /** 明确选了的在线那一家的状态（Key 从哪来、能不能用）。 */
  const chosen = $derived(engine && isOnline(engine) ? providers.find((p) => p.modelId === engine) : undefined);
  const modelConfig = (id: string): Record<string, unknown> => ($settingsStore?.modelConfigs?.[id] ?? {}) as Record<string, unknown>;
  const configText = (id: string, key: string): string => {
    const value = modelConfig(id)[key];
    return typeof value === "string" ? value : "";
  };
  const selectedVoice = $derived.by(() => {
    if (!config) return "";
    if (voicePrefix === "kokoro") return qualifiedVoice("kokoro", config.kokoroVoiceId);
    if (voicePrefix === "system") return qualifiedVoice("system", config.voiceId);
    if (!actualOnline) return "";
    const fallback = providers.find((p) => p.modelId === actualOnline)?.defaultVoice || DEFAULT_VOICE[actualOnline];
    return qualifiedVoice(voicePrefix, configText(actualOnline, "voice") || fallback);
  });
  const voiceOptions = $derived([
    ...(voicePrefix === "system" ? [{ value: "", label: m.companion_speech_voice_system_default() }] : []),
    // 存着的声音不在列表里（列表没拉到、或者是自己填的）也照样显示
    ...(selectedVoice && voicePrefix !== "system" && voicePrefix !== "kokoro" && !voices.some((voice) => voice.id === selectedVoice)
      ? [{ value: selectedVoice, label: selectedVoice.split("/").slice(1).join("/") }] : []),
    ...voices.map((voice) => ({ value: voice.id, label: voice.name })),
  ]);
  const rateSupported = $derived(actualOnline ? (providers.find((p) => p.modelId === actualOnline)?.rate ?? true) : true);
  const progress = $derived(model?.totalBytes ? Math.min(100, Math.round(model.downloadedBytes / model.totalBytes * 100)) : 0);

  function selectEngine(value: string) {
    keyOpen = false; keyInput = ""; checked = null;
    // 在线的几家不写旧字段（旧版本读不懂会把整份配置当作损坏）。
    if (isOnline(value)) return save({ ttsModelId: value });
    if (value === "auto") return save({ speechEngine: "system", ttsModelId: "" });
    if (value === "system") return save({ speechEngine: "system", ttsModelId: "local" });
    return save({ speechEngine: value as AssistantConfig["speechEngine"], ttsModelId: "" });
  }

  async function saveVoice(value: string) {
    if (!config) return;
    // 按声音编号的前缀决定存哪里（“自动”时可能是在线的一家，也可能是系统语音）。
    const prefix = value.includes("/") ? value.split("/")[0] : voicePrefix;
    // 落盘用旧写法（kokoro:zf_017、不带前缀的系统语音），旧版本才能继续使用。
    if (prefix === "kokoro") return save({ kokoroVoiceId: value.startsWith("kokoro/") ? `kokoro:${value.slice(7)}` : value });
    if (prefix === "system") return save({ voiceId: value.startsWith("system/") ? value.slice(7) : value });
    const id = onlineOfPrefix(prefix);
    if (!id) return;
    const native = value.startsWith(`${prefix}/`) ? value.slice(prefix.length + 1) : value;
    await saveConfig(id, { voice: native }, m.companion_speech_voice_saved());
  }

  async function saveConfig(id: string, patch: Record<string, string>, message = m.companion_speech_saved()) {
    await patchSettings({ modelConfigs: { [id]: { ...modelConfig(id), ...patch } } }, message);
    await refreshProviders();
  }

  async function refreshProviders() {
    try {
      const [list, now] = await Promise.all([listSpeechProviders(), getSpeechCurrent()]);
      if (!disposed) { providers = list; current = now; }
    } catch (cause) { if (!disposed) error = errText(cause); }
  }

  async function submitKey() {
    if (!engine || !isOnline(engine) || !keyInput.trim() || checking) return;
    checking = true; checked = null;
    try {
      checked = await saveSpeechKey(engine, keyInput);
      if (checked.saved) { keyInput = ""; keyOpen = false; }
      await refreshProviders();
      await refreshVoices();
    } catch (cause) { checked = { ok: false, saved: false, message: errText(cause), region: null }; }
    finally { checking = false; }
  }

  async function removeKey() {
    if (!engine || !isOnline(engine)) return;
    try {
      await clearSpeechKey(engine);
      checked = null;
      toast(true, m.companion_speech_key_cleared());
      await refreshProviders();
    } catch (cause) { toast(false, errText(cause)); }
  }

  async function preview() {
    if (!selectedVoice || trying) return;
    trying = true;
    try { await tryVoice(selectedVoice); }
    catch (cause) { toast(false, m.companion_speech_try_failed({ error: errText(cause) })); }
    finally { trying = false; }
  }

  let voicesRevision = 0;
  async function refreshVoices() {
    const revision = ++voicesRevision;
    try {
      const items = await listSpeechVoices();
      if (disposed || revision !== voicesRevision) return;
      voices = !isTauri && (voicePrefix === "system") ? [] : (items ?? []);
      voiceError = "";
    } catch (cause) { if (!disposed && revision === voicesRevision) voiceError = errText(cause); }
  }

  // 换了服务商、她的连接或 Key：重新读这一家的状态和声音（换了服务商才清掉旧列表，免得改个声音也闪一下）
  let lastEngine: string | undefined;
  $effect(() => {
    void engine; void config?.ttsModelId; void config?.speechEngine; void $settingsStore?.connections;
    if (engine !== lastEngine) { lastEngine = engine; voices = []; }
    void refreshProviders().then(() => refreshVoices());
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      const cleanup = await subscribeSpeechModel((status) => {
        if (!disposed) { modelRevision++; model = status; }
      });
      if (disposed) { cleanup(); return; }
      unlisten = cleanup;
      await refreshModel();
    })().catch((cause) => { if (!disposed) error = errText(cause); });
    return () => { disposed = true; unlisten?.(); };
  });

  async function refreshModel() {
    const revision = modelRevision;
    const status = await getSpeechModelStatus();
    if (!disposed && revision === modelRevision) model = status;
  }

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({assistant: patch}, m.companion_speech_saved()); }
    finally { saving = false; }
  }

  async function download() {
    if (starting || model?.downloading) return;
    starting = true;
    error = "";
    const revision = modelRevision;
    try {
      const status = await downloadSpeechModel();
      if (!disposed && revision === modelRevision) model = status;
    }
    catch (cause) { if (!disposed) error = errText(cause); }
    finally {
      if (!disposed) {
        starting = false;
        try { await refreshModel(); } catch { /* Preserve the download error. */ }
      }
    }
  }
  async function cancelDownload() {
    cancelling = true;
    try { await cancelSpeechModelDownload(); }
    catch (cause) { if (!disposed) error = errText(cause); }
    finally { if (!disposed) cancelling = false; }
  }

  const engineOptions = $derived((["auto", "system", "kokoro", ...ONLINE] as Engine[])
    .map((value) => ({ value, label: LABELS[value]() })));
  const regionLabel = (region: "cn" | "intl" | null | undefined) =>
    region === "intl" ? m.companion_speech_region_intl() : region === "cn" ? m.companion_speech_region_cn() : "";
  const inputClass = "h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30";
</script>

{#if config}
  <div class="space-y-4 py-4" data-testid="speech-settings">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <p class="m-0 text-[13px] font-medium text-foreground">{m.companion_speech_title()}</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.companion_speech_desc()}</p>
      </div>
      <div class="w-64 max-w-full"><SelectField ariaLabel={m.companion_speech_engine_aria()} value={engine ?? "auto"} disabled={saving}
        options={engineOptions} onchange={(value) => void selectEngine(value)} /></div>
    </div>

    {#if engine === "kokoro"}
      <div class="rounded-xl border border-primary/25 bg-primary/5 p-3 text-xs" data-testid="speech-model">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div><p class="m-0 font-semibold text-foreground">{m.companion_speech_kokoro_title()} <span class="ml-2 font-normal text-primary">{model ? phases[model.phase] : m.companion_speech_loading()}</span></p>
            <p class="mt-1 mb-0 text-[11px] leading-relaxed text-muted-foreground">{m.companion_speech_kokoro_desc()}</p></div>
          {#if model?.downloading || starting}
            <Button variant="outline" size="sm" disabled={cancelling} onclick={cancelDownload}>{cancelling ? m.companion_speech_cancelling() : m.companion_speech_cancel_download()}</Button>
          {:else if model && !model.installed}
            <Button size="sm" disabled={!isTauri} onclick={download}>{m.companion_speech_download()}</Button>
          {/if}
        </div>
        {#if model?.downloading}
          <progress class="mt-3 h-1.5 w-full accent-primary" max="100" value={progress} aria-label={m.companion_speech_progress_aria()}></progress>
          <p class="mt-1 mb-0 text-[10px] text-muted-foreground">{phases[model.phase]} · {progress}%</p>
        {/if}
        {#if model?.error}<p class="mt-2 mb-0 text-destructive" role="alert">{model.error}</p>{/if}
      </div>
    {:else if engine === "auto"}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="auto-speech-info">
        {actualOnline ? m.companion_speech_auto_now({ name: NAMES[actualOnline]() }) : m.companion_speech_auto_system()}
      </p>
    {:else if engine && isOnline(engine)}
      <!-- 在线的一家：Key（就地填，先试一次再存）和这一家要的别的东西 -->
      <div class="space-y-2.5 rounded-xl border border-hairline bg-muted/40 p-3" data-testid="online-speech-info">
        {#if engine === "openai-compat-tts"}
          <label class="block text-xs text-muted-foreground"><span>{m.companion_speech_custom_url()}</span>
            <input class="mt-1 {inputClass}" data-testid="speech-custom-url" placeholder={m.companion_speech_custom_url_placeholder()} value={configText(engine, "baseUrl")}
              onchange={(event) => void saveConfig("openai-compat-tts", { baseUrl: event.currentTarget.value.trim() })} /></label>
          <div class="grid gap-2.5 sm:grid-cols-2">
            <label class="block text-xs text-muted-foreground"><span>{m.companion_speech_custom_model()}</span>
              <input class="mt-1 {inputClass}" data-testid="speech-custom-model" value={configText(engine, "model")}
                onchange={(event) => void saveConfig("openai-compat-tts", { model: event.currentTarget.value.trim() })} /></label>
            <label class="block text-xs text-muted-foreground"><span>{m.companion_speech_custom_voice()}</span>
              <input class="mt-1 {inputClass}" data-testid="speech-custom-voice" value={configText(engine, "voice")}
                onchange={(event) => void saveConfig("openai-compat-tts", { voice: event.currentTarget.value.trim() })} /></label>
          </div>
        {/if}
        {#if engine === "volcano-tts"}
          <label class="block text-xs text-muted-foreground"><span>{m.companion_speech_app_id()}</span>
            <input class="mt-1 {inputClass}" data-testid="speech-volcano-app-id" value={configText(engine, "appId")}
              onchange={(event) => void saveConfig("volcano-tts", { appId: event.currentTarget.value.trim() })} /></label>
        {/if}
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="text-xs font-medium text-foreground">{engine === "volcano-tts" ? m.companion_speech_access_token() : m.companion_speech_key_label()}</span>
          {#if chosen?.apiKeyUrl}
            <button type="button" class="text-xs font-medium text-primary hover:underline" onclick={() => void openExternal(chosen!.apiKeyUrl!)}>
              {m.companion_speech_key_get({ name: NAMES[engine]() })}</button>
          {/if}
        </div>
        {#if chosen && !keyOpen && chosen.keySource.kind !== "none" && chosen.keySource.kind !== "notNeeded"}
          <div class="flex flex-wrap items-center justify-between gap-2" data-testid="speech-key-source">
            <span class="text-xs text-success">
              {chosen.keySource.kind === "own" ? m.companion_speech_key_saved()
                : chosen.keySource.kind === "connection" ? m.companion_speech_key_from_connection({ name: chosen.keySource.name })
                : m.companion_speech_key_from_asr()}{chosen.region ? ` · ${regionLabel(chosen.region)}` : ""}
            </span>
            <span class="flex gap-1.5">
              <Button size="xs" variant="outline" onclick={() => { keyOpen = true; checked = null; }}>{m.companion_speech_key_change()}</Button>
              {#if chosen.keySource.kind === "own"}<Button size="xs" variant="ghost" onclick={() => void removeKey()}>{m.companion_speech_key_clear()}</Button>{/if}
            </span>
          </div>
        {:else}
          <form class="flex gap-2" onsubmit={(event) => { event.preventDefault(); void submitKey(); }}>
            <input type="password" autocomplete="off" class={inputClass} data-testid="speech-key-input" aria-label={m.companion_speech_key_label()}
              placeholder={m.companion_speech_key_placeholder({ name: NAMES[engine]() })} bind:value={keyInput} disabled={checking} />
            <Button size="sm" type="submit" disabled={checking || !keyInput.trim()}>{checking ? m.companion_speech_key_checking() : m.companion_speech_key_save()}</Button>
          </form>
          {#if engine === "openai-compat-tts"}<p class="m-0 text-[11px] text-muted-foreground">{m.companion_speech_key_optional()}</p>{/if}
        {/if}
        {#if checked}
          <p class="m-0 text-xs {checked.ok ? 'text-success' : 'text-destructive'}" role={checked.ok ? "status" : "alert"} data-testid="speech-key-check">
            {checked.message}{checked.region ? ` · ${regionLabel(checked.region)}` : ""}</p>
        {:else if chosen && !chosen.ready && chosen.reason && chosen.keySource.kind !== "none"}
          <p class="m-0 text-xs text-warning" data-testid="speech-not-ready">{m.companion_speech_not_ready({ reason: chosen.reason })}</p>
        {/if}
      </div>
    {:else}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="system-speech-info">{m.companion_speech_system_info()}</p>
    {/if}

    <div class="flex flex-wrap items-center justify-between gap-3">
      <span class="text-[13px] text-foreground/85">{m.companion_speech_voice_label()}</span>
      <div class="flex w-80 max-w-full items-center gap-2">
        {#if engine === "openai-compat-tts"}
          <span class="flex-1 truncate text-xs text-muted-foreground">{configText(engine, "voice")}</span>
        {:else}
          <div class="min-w-0 flex-1"><SelectField ariaLabel={m.companion_speech_voice_label()} value={selectedVoice} disabled={saving}
            options={voiceOptions} onchange={(value) => void saveVoice(value)} /></div>
        {/if}
        <Button size="sm" variant="outline" disabled={trying || !selectedVoice} onclick={() => void preview()} data-testid="speech-try">
          {trying ? m.companion_speech_trying() : m.companion_speech_try()}</Button>
      </div>
    </div>
    {#if actualOnline}
      <p class="m-0 text-[11px] text-muted-foreground">{m.companion_speech_try_hint()}</p>
    {/if}
    {#if voiceError}<p class="m-0 text-xs text-destructive" role="alert">{voiceError}</p>{/if}
    {#if error}<p class="m-0 text-xs text-destructive" role="alert">{error}</p>{/if}

    <div class="grid gap-5 sm:grid-cols-2">
      <label class="block text-xs text-muted-foreground"><span class="flex justify-between"><span>{m.companion_speech_volume()}</span><span class="text-foreground/85">{config.speechVolume}%</span></span>
        <input class="mt-3 block w-full accent-primary" aria-label={m.companion_speech_volume_aria()} type="range" min="0" max="100" step="5" disabled={saving} value={config.speechVolume} onchange={(event) => save({speechVolume:Number(event.currentTarget.value)})} /></label>
      <label class="block text-xs text-muted-foreground"><span class="flex justify-between"><span>{m.companion_speech_rate()}</span><span class="text-foreground/85">{config.speechRate === 0 ? m.companion_speech_rate_normal() : config.speechRate > 0 ? `+${config.speechRate}` : config.speechRate}</span></span>
        <input class="mt-3 block w-full accent-primary" aria-label={m.companion_speech_rate_aria()} type="range" min="-10" max="10" step="1" disabled={saving || !rateSupported} value={config.speechRate} onchange={(event) => save({speechRate:Number(event.currentTarget.value)})} /></label>
    </div>
    {#if !rateSupported}<p class="m-0 text-[11px] text-muted-foreground">{m.companion_speech_no_rate()}</p>{/if}

    <!-- 怎么计费（ADR 048：页面写明在用哪家、怎么计费） -->
    <p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="speech-billing">
      {#if actualOnline}{m.companion_speech_sends_text({ name: NAMES[actualOnline]() })}{BILLING[actualOnline]()}{:else}{m.companion_speech_billing_local()}{/if}
    </p>
    {#if engine === "minimax-tts"}
      <details class="text-xs text-muted-foreground" data-testid="speech-more">
        <summary class="cursor-pointer select-none">{m.companion_speech_more()}</summary>
        <div class="mt-2 flex flex-wrap items-center justify-between gap-3">
          <span>{m.companion_speech_model_label()}</span>
          <div class="w-64 max-w-full"><SelectField ariaLabel={m.companion_speech_model_label()} value={configText("minimax-tts", "model") || "speech-2.8-turbo"}
            options={[{ value: "speech-2.8-turbo", label: m.companion_speech_minimax_turbo() }, { value: "speech-2.8-hd", label: m.companion_speech_minimax_hd() }]}
            onchange={(value) => void saveConfig("minimax-tts", { model: value })} /></div>
        </div>
      </details>
    {/if}
  </div>
{/if}
