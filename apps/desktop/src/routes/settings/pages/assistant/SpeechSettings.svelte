<script lang="ts">
  import { onMount } from "svelte";
  import Select from "../../../../lib/components/Select.svelte";
  import { isTauri, type AssistantConfig } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import {
    cancelSpeechModelDownload, downloadSpeechModel, getSpeechModelStatus,
    listSpeechVoices, subscribeSpeechModel, type SpeechModelStatus, type SpeechVoice,
  } from "../../../../lib/assistant";

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
  const progress = $derived(model?.totalBytes ? Math.min(100, Math.round(model.downloadedBytes / model.totalBytes * 100)) : 0);

  $effect(() => {
    const engine = config?.speechEngine;
    let active = true;
    voices = [];
    if (engine) void listSpeechVoices().then((items) => {
      if (!active) return;
      voices = !isTauri && engine === "system" ? [] : items;
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
  <div class="space-y-4 border-t border-white/8 pt-4" data-testid="speech-settings">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <h3 class="text-sm text-white/85">语音播报</h3>
      <div class="w-64"><Select ariaLabel="语音引擎" value={config.speechEngine} disabled={saving}
        options={[{value:"system",label:"Windows · 系统语音"},{value:"kokoro",label:"Kokoro · 本地中文语音"}]}
        onchange={(value) => void save({speechEngine:value as AssistantConfig["speechEngine"]})} /></div>
    </div>
    {#if config.speechEngine === "kokoro"}
      <div class="rounded-xl border border-kotone-cyan/15 bg-kotone-cyan/5 p-3 text-xs" data-testid="speech-model">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div><p class="font-semibold text-white/80">Kokoro 中文语音 <span class="ml-2 font-normal text-kotone-cyan">{model ? phases[model.phase] : "正在读取…"}</span></p>
            <p class="mt-1 text-[11px] leading-relaxed text-white/45">约 147 MB，下载后本地离线使用。</p></div>
          {#if model?.downloading || starting}
            <button type="button" class="action" disabled={cancelling} onclick={cancelDownload}>{cancelling ? "正在取消…" : "取消下载"}</button>
          {:else if model && !model.installed}
            <button type="button" class="action" disabled={!isTauri} onclick={download}>下载语音模型</button>
          {/if}
        </div>
        {#if model?.downloading}
          <progress class="mt-3 h-1.5 w-full accent-cyan-300" max="100" value={progress} aria-label="语音模型下载进度"></progress>
          <p class="mt-1 text-[10px] text-white/45">{phases[model.phase]} · {progress}%</p>
        {/if}
        {#if model?.error}<p class="mt-2 text-kotone-pink" role="alert">{model.error}</p>{/if}
      </div>
    {:else}
      <p class="text-[11px] leading-relaxed text-white/45" data-testid="system-speech-info">无需下载。</p>
    {/if}
    <div class="flex flex-wrap items-center justify-between gap-3">
      <span class="text-xs text-white/65">播报声音</span>
      <div class="w-64"><Select ariaLabel="播报声音" value={config.speechEngine === "kokoro" ? config.kokoroVoiceId : config.voiceId} disabled={saving}
        options={[...(config.speechEngine === "system" ? [{value:"",label:"系统默认声音"}] : []), ...voices.map((voice) => ({value:voice.id,label:voice.name}))]}
        onchange={(value) => void save(config.speechEngine === "kokoro" ? {kokoroVoiceId:value} : {voiceId:value})} /></div>
    </div>
    {#if voiceError}<p class="text-xs text-kotone-pink" role="alert">{voiceError}</p>{/if}
    {#if error}<p class="text-xs text-kotone-pink" role="alert">{error}</p>{/if}
    <div class="grid gap-5 sm:grid-cols-2">
      <label class="range-label">音量 <span>{config.speechVolume}%</span><input aria-label="播报音量" type="range" min="0" max="100" step="5" disabled={saving} value={config.speechVolume} onchange={(event) => save({speechVolume:Number(event.currentTarget.value)})} /></label>
      <label class="range-label">语速 <span>{config.speechRate === 0 ? "正常" : config.speechRate > 0 ? `+${config.speechRate}` : config.speechRate}</span><input aria-label="播报语速" type="range" min="-10" max="10" step="1" disabled={saving} value={config.speechRate} onchange={(event) => save({speechRate:Number(event.currentTarget.value)})} /></label>
    </div>
  </div>
{/if}

<style>
  .action { border-radius: .5rem; background: #00e5ff15; border: 1px solid #00e5ff40; padding: .4rem .65rem; color: #00e5ff; font-size: .75rem; }
  .action:disabled { opacity: .45; cursor: not-allowed; }
  .action:focus-visible { outline: 2px solid #00e5ff; outline-offset: 3px; }
  .range-label { color: #ffffff99; font-size: .75rem; }
  .range-label span { float: right; color: #ffffffd9; }
  .range-label input { display: block; width: 100%; margin-top: .75rem; accent-color: #00e5ff; }
</style>
