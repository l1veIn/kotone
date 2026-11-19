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
        if (isDownloadCancelled(error)) toastInfo("已取消下载，还在用在线识别");
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
    }, "已换成本地识别");
  }

  async function switchToOnline() {
    await run(async () => {
      await setActiveModel("deepseek-asr", "deepseek-asr");
      await refreshModels();
    }, "已换回在线识别");
  }

  onDestroy(() => unlisten?.());
</script>

<section id="voice-asr" class="scroll-mt-16 min-w-0" data-testid="asr-card" data-asr-state={asrState}>
  <div class="mb-2.5 px-1">
    <h2 class="m-0 text-sm font-semibold text-foreground">语音识别</h2>
  </div>
  <div class="rounded-2xl border border-hairline bg-card/70 px-4 [&>*+*]:border-t [&>*+*]:border-hairline">
    <div class="flex flex-wrap items-center gap-3 py-3.5">
      <div class="min-w-0 flex-1">
        <p class="m-0 truncate text-[13px] font-medium text-foreground" data-testid="asr-model-name">{activeName || "…"}{local ? " · 本地" : " · 在线"}</p>
        {#if asrState === "online-ready"}
          <p class="mt-0.5 mb-0 text-xs text-success" data-testid="asr-status">打开 Kotone 就能用</p>
        {:else if asrState === "online-preparing"}
          <p class="mt-0.5 mb-0 text-xs text-muted-foreground" data-testid="asr-status">正在准备…</p>
        {:else if asrState === "online-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">没准备好：{rt?.error}</p>
        {:else if asrState === "local-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">{LOCAL_ASR_LABELS["local-error"]}：{rt?.error}</p>
        {:else if local}
          <p class="mt-0.5 mb-0 text-xs {asrState === 'local-running' ? 'text-success' : 'text-muted-foreground'}" data-testid="asr-status">
            {asrState === "local-running" ? "● " : ""}{LOCAL_ASR_LABELS[asrState as keyof typeof LOCAL_ASR_LABELS] ?? ""}{asrState === "local-starting" ? "…" : ""}
            {#if asrState === "local-running" && rt?.restartNeeded} · 改了识别设置，重新启动后用新设置{/if}
          </p>
        {/if}
      </div>
      <div class="flex shrink-0 items-center gap-2">
        {#if asrState === "online-error"}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-retry" onclick={() => void run(startRuntime)}>重试</Button>
        {:else if asrState === "local-stopped" || asrState === "local-error"}
          <Button size="sm" disabled={acting || localMissing} data-testid="asr-start" onclick={() => void run(startRuntime, "本地语音识别已启动")}>{asrState === "local-error" ? "重试" : "启动"}</Button>
        {:else if asrState === "local-starting"}
          <Button size="sm" variant="outline" disabled data-testid="asr-start">准备中…</Button>
        {:else if asrState === "local-running"}
          {#if rt?.restartNeeded}
            <Button size="sm" disabled={acting} data-testid="asr-restart" onclick={() => void run(startRuntime, "已按新设置重新启动")}>重新启动</Button>
          {/if}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-stop" onclick={() => void run(stopRuntime, "本地语音识别已停止")}>停止</Button>
        {/if}
      </div>
    </div>

    {#if local}
      {#if localMissing && active}
        <div class="flex flex-wrap items-center gap-3 py-3.5">
          <p class="m-0 min-w-0 flex-1 text-xs text-muted-foreground">{activeName} 还没下载。</p>
          <Button size="sm" disabled={downloading} data-testid="asr-download-active" onclick={() => void switchToLocal(active)}>{downloading ? "下载中…" : "下载"}</Button>
        </div>
      {/if}
      <div class="py-3.5" data-testid="asr-toggle-hotkey">
        <p class="m-0 text-[13px] font-medium text-foreground">启动快捷键</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">在游戏里也能启动 / 停止本地语音识别；没启动时也有效。</p>
        <HotkeyField id="runtime-toggle" title="启动 / 停止本地语音识别快捷键" value={$settingsStore?.runtimeToggleHotkey ?? ""} placeholder="如 Ctrl+Alt+K" compact
          onSave={async (key) => { settingsStore.set(await updateSettings({ runtimeToggleHotkey: key })); }} />
      </div>
      <div class="py-3.5">
        <ToggleField checked={$settingsStore?.ui.autoStart ?? false} label="打开 Kotone 时自动启动"
          desc="自动加载本地识别模型，不用每次点「启动」。"
          onchange={(on) => void patchSettings({ ui: { autoStart: on } }, on ? "打开 Kotone 时会自动启动本地识别" : "已关闭自动启动")} />
      </div>
      <div class="flex flex-wrap items-center justify-between gap-2 py-3">
        {#if hasDeepSeek}
          <button class="text-xs text-primary hover:underline disabled:opacity-50" disabled={acting} data-testid="asr-switch-online" onclick={() => void switchToOnline()}>换回在线识别（DeepSeek）</button>
        {:else}
          <span class="text-xs text-muted-foreground">在线识别（DeepSeek）：需要 DeepSeek Key → <button class="text-primary hover:underline" onclick={onConfigure}>让 Kotone 帮我配置</button></span>
        {/if}
        <button class="text-xs text-primary hover:underline" data-testid="asr-all-models" onclick={onOpenModels}>全部模型 →</button>
      </div>
    {:else}
      <div class="flex flex-wrap items-center gap-3 py-3.5" data-testid="asr-switch-local">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] text-foreground">换成本地识别：<b>免费、离线、边说边出字</b></p>
          {#if recommendedLocal}
            <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{recommendedLocal.displayName.replace(/（.*$/, "")} · 约 {Math.round(recommendedLocal.sizeBytes / 1_000_000)} MB · 模型路径需要纯英文</p>
          {/if}
        </div>
        {#if recommendedLocal}
          <Button size="sm" variant="outline" disabled={downloading || acting} data-testid="asr-switch-local-button"
            onclick={() => void switchToLocal(recommendedLocal)}>{downloading ? "下载中…" : recommendedLocal.downloaded ? "切换" : "下载并切换"}</Button>
        {/if}
      </div>
      {#if downloading}
        <div class="flex items-center gap-2 py-3">
          <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted/70"><div class="h-full rounded-full bg-primary transition-[width] {percent === null ? 'w-1/3 animate-pulse' : ''}" style:width={percent === null ? undefined : `${percent}%`}></div></div>
          <span class="w-9 text-right text-[10px] text-muted-foreground">{percent === null ? "连接" : `${percent}%`}</span>
          <button class="text-[11px] text-muted-foreground hover:text-foreground disabled:opacity-50" disabled={cancelling}
            onclick={() => { cancelling = true; void cancelDownload().catch(() => (cancelling = false)); }}>{cancelling ? "正在取消…" : "取消"}</button>
        </div>
      {/if}
      {#if downloadError}<p class="m-0 py-3 text-xs text-destructive">{downloadError}</p>{/if}
      <div class="flex justify-end py-3">
        <button class="text-xs text-primary hover:underline" data-testid="asr-all-models" onclick={onOpenModels}>全部模型 →</button>
      </div>
    {/if}
  </div>
</section>

{#if manualGuide}
  <ManualDownloadDialog guide={manualGuide} error={downloadError} onClose={() => (manualGuide = null)}
    onRetry={() => { manualGuide = null; if (recommendedLocal) void switchToLocal(recommendedLocal); }}
    onRecheck={() => { manualGuide = null; void refreshModels(); }} />
{/if}
