<script lang="ts">
  import { onMount } from "svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { isTauri, type AssistantConfig } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import {
    cancelSpeechModelDownload, downloadSpeechModel, getSpeechModelStatus,
    listSpeechVoices, subscribeSpeechModel, type SpeechModelStatus, type SpeechVoice,
  } from "../../../../lib/assistant";

  let { legacyReminder = false }: { legacyReminder?: boolean } = $props();
  let model = $state<SpeechModelStatus | null>(null);
  let voices = $state<SpeechVoice[]>([]);
  let error = $state("");
  let voiceError = $state("");
  let saving = $state(false);
  let starting = $state(false);
  let cancelling = $state(false);
  let disposed = false;
  let modelRevision = 0;
  const config = $derived($settingsStore?.assistant);
  const phases: Record<SpeechModelStatus["phase"], string> = {
    idle: "尚未下载", downloading: "正在下载", verifying: "正在校验", extracting: "正在安装",
    installed: "已就绪", error: "安装失败", cancelled: "已取消下载",
  };
  /** 在线引擎对应的统一模型清单 ID（声音存在 modelConfigs[id].voice） */
  const onlineModel: Record<string, string> = { openai: "openai-compat-tts", volcano: "volcano-tts", deepseek: "deepseek-tts" };
  type Engine = "auto" | "system" | "kokoro" | "openai" | "volcano" | "deepseek";
  /** 声音编号统一写成「提供方/声音」；旧配置里不带前缀的值照常匹配。 */
  function qualifiedVoice(engine: string, id: string): string {
    const value = id.trim();
    if (!value) return "";
    if (value.startsWith(`${engine}/`)) return value;
    if (engine === "kokoro" && value.startsWith("kokoro:")) return `kokoro/${value.slice(7)}`;
    return `${engine}/${value}`;
  }
  /**
   * 选中的引擎：在线引擎记在新字段 ttsModelId，本地引擎在旧字段 speechEngine。
   * ttsModelId 为空且本地是系统语音 = “自动”（默认）：琴音用 DeepSeek 连接时用 DeepSeek 语音，否则系统语音；
   * 明确选了系统语音写 ttsModelId = "local"。
   */
  const engine = $derived<Engine | undefined>(
    !config ? undefined
      : legacyReminder ? config.speechEngine
      : config.ttsModelId === onlineModel.openai ? "openai"
      : config.ttsModelId === onlineModel.volcano ? "volcano"
      : config.ttsModelId === onlineModel.deepseek ? "deepseek"
      : config.speechEngine === "kokoro" ? "kokoro"
      : config.ttsModelId === "local" ? "system"
      : "auto",
  );
  /** “自动”时实际在用的声音提供方（从声音列表的前缀看出来）。 */
  const autoProvider = $derived(engine === "auto" ? (voices[0]?.id.split("/")[0] ?? "") : "");
  function selectEngine(value: string) {
    if (legacyReminder) return save({ speechEngine: value === "kokoro" ? "kokoro" : "system" });
    // 在线引擎不写旧字段（旧版本读不懂会把整份配置当作损坏）。
    if (value === "openai" || value === "volcano" || value === "deepseek") return save({ ttsModelId: onlineModel[value] });
    if (value === "auto") return save({ speechEngine: "system", ttsModelId: "" });
    if (value === "system") return save({ speechEngine: "system", ttsModelId: "local" });
    return save({ speechEngine: value as AssistantConfig["speechEngine"], ttsModelId: "" });
  }
  /** 声音属于哪个提供方：“自动”时看实际在用的那个。 */
  const voiceEngine = $derived(engine === "auto" ? (autoProvider || "system") : engine);
  const selectedVoice = $derived.by(() => {
    if (!config || !voiceEngine) return "";
    if (voiceEngine === "kokoro") return qualifiedVoice(voiceEngine, config.kokoroVoiceId);
    if (voiceEngine === "system") return qualifiedVoice(voiceEngine, config.voiceId);
    const model = onlineModel[voiceEngine];
    if (!model) return "";
    const stored = $settingsStore?.modelConfigs?.[model]?.voice;
    const fallback = voiceEngine === "deepseek" ? "stella" : "";
    return qualifiedVoice(voiceEngine, typeof stored === "string" && stored ? stored : fallback);
  });
  async function saveVoice(value: string) {
    if (!config) return;
    // 按声音编号的前缀决定存哪里（“自动”时可能是 DeepSeek，也可能是系统语音）。
    const provider = value.includes("/") ? value.split("/")[0] : (voiceEngine ?? "system");
    // 落盘用旧写法（kokoro:zf_017、不带前缀的系统语音），旧版本才能继续使用。
    if (provider === "kokoro") return save({ kokoroVoiceId: value.startsWith("kokoro/") ? `kokoro:${value.slice(7)}` : value });
    if (provider === "system") return save({ voiceId: value.startsWith("system/") ? value.slice(7) : value });
    const model = onlineModel[provider];
    if (!model) return;
    const current = $settingsStore?.modelConfigs?.[model] ?? {};
    const native = value.startsWith(`${provider}/`) ? value.slice(provider.length + 1) : value;
    await patchSettings({ modelConfigs: { [model]: { ...current, voice: native } } }, "播报声音已保存");
  }
  const progress = $derived(model?.totalBytes ? Math.min(100, Math.round(model.downloadedBytes / model.totalBytes * 100)) : 0);

  $effect(() => {
    const current = engine;
    let active = true;
    voices = [];
    if (current) void listSpeechVoices(legacyReminder || undefined).then((items) => {
      if (!active) return;
      voices = !isTauri && (current === "system" || current === "auto") ? [] : (items ?? []);
      voiceError = "";
    }).catch((cause) => { if (active) voiceError = errText(cause); });
    return () => { active = false; };
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
    try { await patchSettings({assistant: patch}, "播报设置已保存"); }
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
</script>

{#if config}
  <div class="space-y-4 py-4" data-testid="speech-settings">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <p class="m-0 text-[13px] font-medium text-foreground">语音播报</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">提醒、回答和自动化结果用这个声音读出来。</p>
      </div>
      <div class="w-64 max-w-full"><SelectField ariaLabel="语音引擎" value={engine ?? "auto"} disabled={saving}
        options={legacyReminder
          ? [{value:"system",label:"Windows · 系统语音"},{value:"kokoro",label:"Kokoro · 本地中文语音"}]
          : [{value:"auto",label:"自动（默认）"},{value:"deepseek",label:"在线 · DeepSeek 语音"},{value:"system",label:"Windows · 系统语音"},{value:"kokoro",label:"Kokoro · 本地中文语音"},{value:"openai",label:"在线 · OpenAI 兼容"},{value:"volcano",label:"在线 · 火山语音合成"}]}
        onchange={(value) => void selectEngine(value)} /></div>
    </div>
    {#if engine === "kokoro"}
      <div class="rounded-xl border border-primary/25 bg-primary/5 p-3 text-xs" data-testid="speech-model">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div><p class="m-0 font-semibold text-foreground">Kokoro 中文语音 <span class="ml-2 font-normal text-primary">{model ? phases[model.phase] : "正在读取…"}</span></p>
            <p class="mt-1 mb-0 text-[11px] leading-relaxed text-muted-foreground">约 147 MB，下载后本地离线使用。</p></div>
          {#if model?.downloading || starting}
            <Button variant="outline" size="sm" disabled={cancelling} onclick={cancelDownload}>{cancelling ? "正在取消…" : "取消下载"}</Button>
          {:else if model && !model.installed}
            <Button size="sm" disabled={!isTauri} onclick={download}>下载语音模型</Button>
          {/if}
        </div>
        {#if model?.downloading}
          <progress class="mt-3 h-1.5 w-full accent-primary" max="100" value={progress} aria-label="语音模型下载进度"></progress>
          <p class="mt-1 mb-0 text-[10px] text-muted-foreground">{phases[model.phase]} · {progress}%</p>
        {/if}
        {#if model?.error}<p class="mt-2 mb-0 text-destructive" role="alert">{model.error}</p>{/if}
      </div>
    {:else if engine === "auto"}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="auto-speech-info">
        {autoProvider === "deepseek"
          ? "正在用 DeepSeek 语音（AI 连接里的 DeepSeek）：按字数计费，每次播报都会把文字发给 DeepSeek，相同的话复用缓存；不能调语速。"
          : "AI 连接是 DeepSeek（且有密钥）时自动用 DeepSeek 语音，否则用 Windows 系统语音。"}
      </p>
    {:else if engine === "deepseek"}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="online-speech-info">
        DeepSeek 语音：默认用 AI 连接里的 DeepSeek 与密钥，按字数计费（汉字算 2 个字符），每次播报都会把文字发给 DeepSeek；相同的话复用缓存，不能调语速。连接可在「设置 → 模型 → 语音合成」里另选。
      </p>
    {:else if engine === "openai" || engine === "volcano"}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="online-speech-info">
        在线语音：每次播报都会把文字发给服务商。连接、模型与声音在「设置 → 模型 → 语音合成」里配置；相同的话会直接复用缓存，不再请求。
      </p>
    {:else}
      <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="system-speech-info">无需下载。</p>
    {/if}
    <div class="flex flex-wrap items-center justify-between gap-3">
      <span class="text-[13px] text-foreground/85">播报声音</span>
      <div class="w-64 max-w-full"><SelectField ariaLabel="播报声音" value={selectedVoice} disabled={saving}
        options={[...(voiceEngine === "system" ? [{value:"",label:"系统默认声音"}] : []), ...(selectedVoice && !voices.some((voice) => voice.id === selectedVoice) && voiceEngine !== "system" && voiceEngine !== "kokoro" ? [{value:selectedVoice,label:selectedVoice.split("/").slice(1).join("/")}] : []), ...voices.map((voice) => ({value:voice.id,label:voice.name}))]}
        onchange={(value) => void saveVoice(value)} /></div>
    </div>
    {#if voiceError}<p class="m-0 text-xs text-destructive" role="alert">{voiceError}</p>{/if}
    {#if error}<p class="m-0 text-xs text-destructive" role="alert">{error}</p>{/if}
    <div class="grid gap-5 sm:grid-cols-2">
      <label class="block text-xs text-muted-foreground"><span class="flex justify-between"><span>音量</span><span class="text-foreground/85">{config.speechVolume}%</span></span>
        <input class="mt-3 block w-full accent-primary" aria-label="播报音量" type="range" min="0" max="100" step="5" disabled={saving} value={config.speechVolume} onchange={(event) => save({speechVolume:Number(event.currentTarget.value)})} /></label>
      <label class="block text-xs text-muted-foreground"><span class="flex justify-between"><span>语速</span><span class="text-foreground/85">{config.speechRate === 0 ? "正常" : config.speechRate > 0 ? `+${config.speechRate}` : config.speechRate}</span></span>
        <input class="mt-3 block w-full accent-primary" aria-label="播报语速" type="range" min="-10" max="10" step="1" disabled={saving} value={config.speechRate} onchange={(event) => save({speechRate:Number(event.currentTarget.value)})} /></label>
    </div>
  </div>
{/if}
