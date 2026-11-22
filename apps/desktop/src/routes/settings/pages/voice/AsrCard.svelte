<script lang="ts">
  /*
   * 语音识别卡片（ADR 028 W8 / W9）：在线识别打开就能用，看不到“启动”；
   * 本地识别才有 未启动 / 准备中 / 已启动 / 出问题了，启动按钮、启动快捷键、“打开时自动启动”都收在这里。
   * 换回在线识别时整块收起，启动快捷键和自动启动的值保留。
   */
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    cancelDownload, downloadModel, getModelInstallGuide, isDownloadCancelled, isTauri, listModels, setActiveModel,
    startRuntime, stopRuntime, updateSettings,
    type DownloadProgress, type ModelInfo, type ModelInstallGuide,
  } from "../../../../lib/ipc";
  import { LOCAL_ASR_LABELS } from "../../../../lib/ipc/contracts";
  import { asrStore, runtimeStore } from "../../../../lib/stores/runtime";
  import { errText, patchSettings, settingsStore, toast, toastInfo } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import ToggleField from "../../../../lib/components/settings/ToggleField.svelte";
  import ManualDownloadDialog from "../../../../lib/components/ManualDownloadDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenModels, onConfigure }: {
    /** 「全部模型」：设置 → 模型 → 语音识别。 */
    onOpenModels: () => void;
    /** 在线识别要 DeepSeek Key：让 Kotone 帮我配置。 */
    onConfigure: () => void;
  } = $props();

  let models = $state<ModelInfo[]>([]);
  const asrState = $derived($asrStore);
  const rt = $derived($runtimeStore);
  const local = $derived(asrState.startsWith("local-"));
  const active = $derived(models.find((m) => m.id === rt?.modelId) ?? null);
  const activeName = $derived((active?.displayName ?? rt?.modelId ?? "").replace(/（.*$/, ""));
  const recommendedLocal = $derived(models.find((m) => m.engineId === "sherpa-streaming" && m.backend !== "remote") ?? null);
  const hasDeepSeek = $derived(($settingsStore?.connections ?? []).some((c) => c.provider === "deepseek" || c.baseUrl.includes("api.deepseek.com")));
  const localMissing = $derived(local && active !== null && !active.downloaded);
  let acting = $state(false);

  async function refreshModels() {
    try { models = await listModels(); } catch { /* 卡片只少了模型名 */ }
  }
  onMount(() => { void refreshModels(); });
  $effect(() => { rt?.modelId; void refreshModels(); });

  async function run(action: () => Promise<unknown>, done?: string) {
    if (acting) return;
    acting = true;
    try {
      await action();
      if (done) toast(true, done);
    } catch (error) {
      toast(false, errText(error));
    } finally {
      acting = false;
    }
  }

  // ---------- 换成本地识别：下载并切换 ----------
  let downloading = $state(false);
  let cancelling = $state(false);
  let progress = $state<{ downloaded: number; total: number } | null>(null);
  let downloadError = $state("");
  let manualGuide = $state<ModelInstallGuide | null>(null);
  let unlisten: (() => void) | undefined;
  const percent = $derived(progress && progress.total > 0 ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null);

  async function switchToLocal(model: ModelInfo) {
    if (downloading) return;
    downloadError = "";
    if (!model.downloaded) {
      downloading = true;
      progress = null;
      try {
        if (isTauri) unlisten = await listen<DownloadProgress>("kotone://download", (event) => {
          if (event.payload.id === model.id) progress = { downloaded: event.payload.downloaded, total: event.payload.total };
        });
        await downloadModel(model.id);
      } catch (error) {
        if (isDownloadCancelled(error)) toastInfo(m.voice_asr_download_cancelled());
        else {
          downloadError = errText(error);
          manualGuide = await getModelInstallGuide(model.id).catch(() => null);
        }
        return;
      } finally {
        downloading = false;
        cancelling = false;
        unlisten?.();
        unlisten = undefined;
      }
    }
    // 切到本地后立即启动一次（后端负责），“打开时自动启动”默认勾上（D9）。
    await run(async () => {
      await setActiveModel(model.engineId, model.id);
      settingsStore.set(await updateSettings({ ui: { autoStart: true } }));
      await refreshModels();
    }, m.voice_asr_switched_local());
  }

  async function switchToOnline() {
    await run(async () => {
      await setActiveModel("deepseek-asr", "deepseek-asr");
      await refreshModels();
    }, m.voice_asr_switched_online());
  }

  onDestroy(() => unlisten?.());
</script>

<section id="voice-asr" class="scroll-mt-16 min-w-0" data-testid="asr-card" data-asr-state={asrState}>
  <div class="mb-2.5 px-1">
    <h2 class="m-0 text-sm font-semibold text-foreground">{m.voice_asr_title()}</h2>
  </div>
  <div class="rounded-2xl border border-hairline bg-card/70 px-4 [&>*+*]:border-t [&>*+*]:border-hairline">
    <div class="flex flex-wrap items-center gap-3 py-3.5">
      <div class="min-w-0 flex-1">
        <p class="m-0 truncate text-[13px] font-medium text-foreground" data-testid="asr-model-name">{activeName || "…"} · {local ? m.voice_asr_mode_local() : m.voice_asr_mode_online()}</p>
        {#if asrState === "online-ready"}
          <p class="mt-0.5 mb-0 text-xs text-success" data-testid="asr-status">{m.voice_asr_online_ready()}</p>
        {:else if asrState === "online-preparing"}
          <p class="mt-0.5 mb-0 text-xs text-muted-foreground" data-testid="asr-status">{m.voice_asr_preparing()}</p>
        {:else if asrState === "online-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">{m.voice_asr_online_error({ error: rt?.error ?? "" })}</p>
        {:else if asrState === "local-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">{m.voice_asr_labeled_error({ label: LOCAL_ASR_LABELS["local-error"], error: rt?.error ?? "" })}</p>
        {:else if local}
          <p class="mt-0.5 mb-0 text-xs {asrState === 'local-running' ? 'text-success' : 'text-muted-foreground'}" data-testid="asr-status">
            {asrState === "local-running" ? "● " : ""}{LOCAL_ASR_LABELS[asrState as keyof typeof LOCAL_ASR_LABELS] ?? ""}{asrState === "local-starting" ? "…" : ""}
            {#if asrState === "local-running" && rt?.restartNeeded} {m.voice_asr_restart_hint()}{/if}
          </p>
        {/if}
      </div>
      <div class="flex shrink-0 items-center gap-2">
        {#if asrState === "online-error"}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-retry" onclick={() => void run(startRuntime)}>{m.voice_action_retry()}</Button>
        {:else if asrState === "local-stopped" || asrState === "local-error"}
          <Button size="sm" disabled={acting || localMissing} data-testid="asr-start" onclick={() => void run(startRuntime, m.voice_asr_started())}>{asrState === "local-error" ? m.voice_action_retry() : m.voice_asr_start()}</Button>
        {:else if asrState === "local-starting"}
          <Button size="sm" variant="outline" disabled data-testid="asr-start">{m.voice_asr_starting()}</Button>
        {:else if asrState === "local-running"}
          {#if rt?.restartNeeded}
            <Button size="sm" disabled={acting} data-testid="asr-restart" onclick={() => void run(startRuntime, m.voice_asr_restarted())}>{m.voice_asr_restart()}</Button>
          {/if}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-stop" onclick={() => void run(stopRuntime, m.voice_asr_stopped())}>{m.voice_asr_stop()}</Button>
        {/if}
      </div>
    </div>

    {#if local}
      {#if localMissing && active}
        <div class="flex flex-wrap items-center gap-3 py-3.5">
          <p class="m-0 min-w-0 flex-1 text-xs text-muted-foreground">{m.voice_asr_not_downloaded({ name: activeName })}</p>
          <Button size="sm" disabled={downloading} data-testid="asr-download-active" onclick={() => void switchToLocal(active)}>{downloading ? m.voice_asr_downloading() : m.voice_asr_download()}</Button>
        </div>
      {/if}
      <div class="py-3.5" data-testid="asr-toggle-hotkey">
        <p class="m-0 text-[13px] font-medium text-foreground">{m.voice_asr_toggle_hotkey_title()}</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.voice_asr_toggle_hotkey_desc()}</p>
        <HotkeyField id="runtime-toggle" title={m.voice_hotkey_name_runtime_toggle()} value={$settingsStore?.runtimeToggleHotkey ?? ""} placeholder={m.voice_hotkey_placeholder({ example: "Ctrl+Alt+K" })} compact
          onSave={async (key) => { settingsStore.set(await updateSettings({ runtimeToggleHotkey: key })); }} />
      </div>
      <div class="py-3.5">
        <ToggleField checked={$settingsStore?.ui.autoStart ?? false} label={m.voice_asr_autostart_label()}
          desc={m.voice_asr_autostart_desc()}
          onchange={(on) => void patchSettings({ ui: { autoStart: on } }, on ? m.voice_asr_autostart_on() : m.voice_asr_autostart_off())} />
      </div>
      <div class="flex flex-wrap items-center justify-between gap-2 py-3">
        {#if hasDeepSeek}
          <button class="text-xs text-primary hover:underline disabled:opacity-50" disabled={acting} data-testid="asr-switch-online" onclick={() => void switchToOnline()}>{m.voice_asr_switch_online()}</button>
        {:else}
          <span class="text-xs text-muted-foreground">{m.voice_asr_online_needs_key()} <button class="text-primary hover:underline" onclick={onConfigure}>{m.voice_shell_configure()}</button></span>
        {/if}
        <button class="text-xs text-primary hover:underline" data-testid="asr-all-models" onclick={onOpenModels}>{m.voice_asr_all_models()}</button>
      </div>
    {:else}
      <div class="flex flex-wrap items-center gap-3 py-3.5" data-testid="asr-switch-local">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] text-foreground">{m.voice_asr_switch_local_title()}<b>{m.voice_asr_switch_local_pitch()}</b></p>
          {#if recommendedLocal}
            <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.voice_asr_recommended_info({ name: recommendedLocal.displayName.replace(/（.*$/, ""), size: Math.round(recommendedLocal.sizeBytes / 1_000_000) })}</p>
          {/if}
        </div>
        {#if recommendedLocal}
          <Button size="sm" variant="outline" disabled={downloading || acting} data-testid="asr-switch-local-button"
            onclick={() => void switchToLocal(recommendedLocal)}>{downloading ? m.voice_asr_downloading() : recommendedLocal.downloaded ? m.voice_asr_switch() : m.voice_asr_download_and_switch()}</Button>
        {/if}
      </div>
      {#if downloading}
        <div class="flex items-center gap-2 py-3">
          <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted/70"><div class="h-full rounded-full bg-primary transition-[width] {percent === null ? 'w-1/3 animate-pulse' : ''}" style:width={percent === null ? undefined : `${percent}%`}></div></div>
          <span class="w-9 text-right text-[10px] text-muted-foreground">{percent === null ? m.voice_asr_progress_connecting() : `${percent}%`}</span>
          <button class="text-[11px] text-muted-foreground hover:text-foreground disabled:opacity-50" disabled={cancelling}
            onclick={() => { cancelling = true; void cancelDownload().catch(() => (cancelling = false)); }}>{cancelling ? m.voice_asr_cancelling() : m.voice_action_cancel()}</button>
        </div>
      {/if}
      {#if downloadError}<p class="m-0 py-3 text-xs text-destructive">{downloadError}</p>{/if}
      <div class="flex justify-end py-3">
        <button class="text-xs text-primary hover:underline" data-testid="asr-all-models" onclick={onOpenModels}>{m.voice_asr_all_models()}</button>
      </div>
    {/if}
  </div>
</section>

{#if manualGuide}
  <ManualDownloadDialog guide={manualGuide} error={downloadError} onClose={() => (manualGuide = null)}
    onRetry={() => { manualGuide = null; if (recommendedLocal) void switchToLocal(recommendedLocal); }}
    onRecheck={() => { manualGuide = null; void refreshModels(); }} />
{/if}
