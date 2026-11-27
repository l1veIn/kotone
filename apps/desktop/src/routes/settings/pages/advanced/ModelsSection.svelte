<script lang="ts">
  import { onMount } from "svelte";
  import { slide } from "svelte/transition";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    listSttEngines,
    listModels,
    listConnections,
    modelsDirPathError,
    downloadModel,
    setActiveModel,
    deleteModel,
    getModelsDir,
    setModelsDir,
    openModelsDir,
    openHistoryDir,
    openExternal,
    cancelDownload,
    getModelInstallGuide,
    isDownloadCancelled,
    isTauri,
    startRuntime,
    updateSettings,
    type DownloadProgress,
    type EngineInfo,
    type ConnectionInfo,
    type ModelInfo,
    type ModelInstallGuide,
    type ModelsDirInfo,
  } from "../../../../lib/ipc";
  import { modelSecretSaved } from "../../../../lib/ipc";
  import { openTarget } from "../../../../lib/stores/navigation";
  import { settingsStore, patchSettings, toast, toastInfo, toastWarn, errText } from "../../../../lib/stores/ui";
  import { runtimeStore } from "../../../../lib/stores/runtime";
  import ManualDownloadDialog from "../../../../lib/components/ManualDownloadDialog.svelte";
  import CatalogModelList from "./CatalogModelList.svelte";
  import AsrRuntimeCard from "./AsrRuntimeCard.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { Button } from "$lib/components/ui/button";
  import stickerCurious from "../../../../assets/brand/stickers/curious.webp";
  import { m } from "$lib/paraglide/messages.js";

  let engines = $state<EngineInfo[] | null>(null);
  let models = $state<ModelInfo[]>([]);
  let dirInfo = $state<ModelsDirInfo | null>(null);
  let dlProgress = $state<Record<string, number | null>>({});
  let dlErrors = $state<Record<string, string>>({});
  let confirmingDelete = $state<string | null>(null);
  let deleting = $state<string | null>(null);
  let editingDir = $state(false);
  let dirDraft = $state("");
  let migrating = $state(false);
  let downloadProxyDraft = $state("");
  let cancellingId = $state<string | null>(null);
  let manualGuide = $state<ModelInstallGuide | null>(null);
  let manualGuideError = $state("");
  let connections = $state<ConnectionInfo[]>([]);
  /** 统一模型清单按类别分栏：asr 语音识别（原有列表）/ ocr 文字识别 / tts 语音合成 / search 联网搜索（ADR 026） */
  let { focus = null }: {
    /** 聊天里“去看看 设置 → 模型 → 语音识别”等：切到这个类别。 */
    focus?: { category: string; n: number } | null;
  } = $props();
  let category = $state<"asr" | "ocr" | "tts" | "search">("asr");
  $effect(() => {
    const next = focus?.category;
    if (next === "asr" || next === "ocr" || next === "tts" || next === "search") category = next;
  });
  const categories = [
    { id: "asr", name: m.system_models_category_asr() },
    { id: "ocr", name: m.system_models_category_ocr() },
    { id: "tts", name: m.system_models_category_tts() },
    { id: "search", name: m.system_models_category_search() },
  ] as const;

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);
  const dirDraftError = $derived(modelsDirPathError(dirDraft.trim()));
  const currentModelsDirError = $derived(dirInfo ? modelsDirPathError(dirInfo.dir) : null);
  const historyDirLabel = $derived($settingsStore?.history.dir.trim() || "~/.kotone/history");
  const historyDirCustom = $derived(Boolean($settingsStore?.history.dir.trim()));

  const visibleModels = $derived(models.filter((model) => model.engineId !== "mock-stream"));

  onMount(() => {
    let un: (() => void) | undefined;
    void (async () => {
      await reload();
      downloadProxyDraft = $settingsStore?.download.ghProxy ?? "";
      if (!isTauri) return;
      un = await listen<DownloadProgress>("kotone://download", (ev) => {
        const { id, downloaded, total } = ev.payload;
        if (id in dlProgress) {
          dlProgress[id] = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
        }
      });
    })();
    return () => un?.();
  });

  async function reload() {
    const [en, mo, di, cn] = await Promise.all([
      listSttEngines().catch((e) => {
        console.error("[engine-page] list_stt_engines 失败：", e);
        toast(false, m.system_models_load_engines_failed({ error: errText(e) }));
        return null;
      }),
      listModels().catch((e) => {
        console.error("[engine-page] list_models 失败：", e);
        toast(false, m.system_models_load_models_failed({ error: errText(e) }));
        return null;
      }),
      getModelsDir().catch((e) => {
        console.error("[engine-page] get_models_dir 失败：", e);
        toast(false, m.system_models_load_dir_failed({ error: errText(e) }));
        return null;
      }),
      listConnections().catch((e) => {
        console.error("[engine-page] list_connections 失败：", e);
        return [];
      }),
    ]);
    if (en === null && mo === null && di === null) {
      engines = [];
      return;
    }
    if (en !== null) engines = en;
    if (mo !== null) models = mo;
    if (di !== null) dirInfo = di;
    connections = cn;
  }

  async function onChangeDir() {
    const dir = dirDraft.trim();
    if (migrating || dir === (dirInfo?.isDefault ? "" : dirInfo?.dir)) return;
    const pathError = modelsDirPathError(dir);
    if (pathError) {
      toast(false, pathError);
      return;
    }
    migrating = true;
    try {
      const report = await setModelsDir(dir);
      if (report.failed.length > 0) {
        toastWarn(m.system_models_dir_migrate_partial({ count: report.failed.length, items: report.failed.join(", ") }));
      } else {
        toast(true, report.moved.length > 0 ? m.system_models_dir_changed_moved({ count: report.moved.length }) : m.system_models_dir_changed());
      }
      editingDir = false;
      await reload();
    } catch (e) {
      toast(false, m.system_models_dir_change_failed({ error: errText(e) }));
    } finally {
      migrating = false;
    }
  }

  async function onOpenDir() {
    try {
      await openModelsDir();
    } catch (e) {
      toast(false, m.system_models_open_dir_failed({ error: errText(e) }));
    }
  }

  async function onBrowseDir() {
    const sel = await openDialog({ directory: true, title: m.system_models_dir_dialog_title() }).catch(() => null);
    if (!sel) return;
    const pathError = modelsDirPathError(sel);
    if (pathError) {
      toast(false, pathError);
      return;
    }
    dirDraft = sel;
  }

  async function onChangeHistoryDir() {
    const sel = await openDialog({ directory: true, title: m.system_models_history_dir_dialog_title() }).catch(() => null);
    if (!sel) return;
    await patchSettings({ history: { dir: sel } }, m.system_models_history_dir_changed());
  }

  async function onOpenHistoryDir() {
    try {
      await openHistoryDir();
    } catch (e) {
      toast(false, m.system_models_open_dir_failed({ error: errText(e) }));
    }
  }

  async function onDownload(id: string) {
    if (downloadingAny) return;
    delete dlErrors[id];
    dlProgress[id] = null;
    try {
      await downloadModel(id);
      models = models.map((model) => (model.id === id ? { ...model, downloaded: true } : model));
      toast(true, m.system_catalog_download_done({ name: id }));
      await reload();
    } catch (e) {
      if (isDownloadCancelled(e)) {
        toastInfo(m.system_catalog_download_cancelled());
      } else {
        dlErrors[id] = errText(e);
        toast(false, m.system_models_download_failed_toast());
        await openManualGuide(id, dlErrors[id]);
      }
    } finally {
      delete dlProgress[id];
      if (cancellingId === id) cancellingId = null;
    }
  }

  async function onCancelDownload(id: string) {
    if (cancellingId) return;
    cancellingId = id;
    try {
      await cancelDownload();
    } catch (e) {
      cancellingId = null;
      toast(false, m.system_models_cancel_failed({ error: errText(e) }));
    }
  }

  async function openManualGuide(id: string, error = "") {
    try {
      manualGuide = await getModelInstallGuide(id);
      manualGuideError = error;
    } catch (e) {
      toast(false, m.system_models_guide_load_failed({ error: errText(e) }));
    }
  }

  async function recheckAfterManualInstall() {
    const id = manualGuide?.id;
    await reload();
    if (id && models.some((model) => model.id === id && model.downloaded)) {
      delete dlErrors[id];
      manualGuide = null;
      manualGuideError = "";
      toast(true, m.system_models_recheck_found());
    } else {
      toastWarn(m.system_models_recheck_missing());
    }
  }

  async function onDelete(id: string) {
    if (deleting) return;
    deleting = id;
    confirmingDelete = null;
    try {
      const outcome = await deleteModel(id);
      models = models.map((model) => (model.id === id ? { ...model, downloaded: false } : model));
      if (outcome.wasActive) {
        settingsStore.update((s) => {
          if (!s) return s;
          const next = structuredClone(s);
          for (const opts of Object.values(next.engineOptions)) {
            const o = opts as Record<string, unknown>;
            if (o.model === id) delete o.model;
          }
          return next;
        });
        toast(true, m.system_models_deleted_was_active({ name: id }));
      } else {
        toast(true, m.system_catalog_deleted({ name: id }));
      }
      await reload();
    } catch (e) {
      toast(false, m.system_catalog_delete_failed({ error: errText(e) }));
    } finally {
      deleting = null;
    }
  }

  async function onSetActive(engineId: string, modelId: string) {
    // 从在线换成本地：后端立即启动一次，“打开 Kotone 时自动启动”默认勾上（D9；原来在语音输入页的识别卡片里做）
    const onlineToLocal = $runtimeStore?.asrLocal === false && models.find((model) => model.id === modelId)?.backend !== "remote";
    try {
      await setActiveModel(engineId, modelId);
      if (onlineToLocal && !$settingsStore?.ui.autoStart) settingsStore.set(await updateSettings({ ui: { autoStart: true } }));
      settingsStore.update((s) => {
        if (!s) return s;
        const next = structuredClone(s);
        next.activeModelId = modelId;
        next.sttEngine = engineId;
        ((next.engineOptions[engineId] ??= {}) as Record<string, unknown>).model = modelId;
        return next;
      });
      if ($runtimeStore?.phase === "running") {
        toastWarn(m.system_models_activated_restart());
      } else {
        toast(true, m.system_models_activated());
      }
    } catch (e) {
      toast(false, m.system_models_activate_failed({ error: errText(e) }));
    }
  }

  async function saveDownloadProxy() {
    await patchSettings(
      { download: { ghProxy: downloadProxyDraft.trim() } },
      m.system_models_proxy_saved(),
    );
  }

  function formatSize(bytes: number): string {
    if (bytes <= 0) return m.system_catalog_size_none();
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  function isCurrentModel(modelId: string): boolean {
    const selected = $settingsStore?.activeModelId?.trim();
    if (selected) return selected === modelId;
    const engineId = $settingsStore?.sttEngine;
    if (!engineId) return false;
    const configured = ($settingsStore?.engineOptions?.[engineId] as Record<string, unknown> | undefined)
      ?.model as string | undefined;
    if (configured) return configured === modelId;
    const defaultModel = models.find((model) => model.engineId === engineId);
    return defaultModel?.id === modelId;
  }

  function fieldValue(modelId: string, key: string, fallback: string): string {
    const stored = $settingsStore?.modelConfigs?.[modelId]?.[key];
    return typeof stored === "string" ? stored : fallback;
  }

  async function saveField(modelId: string, key: string, value: string) {
    const current = $settingsStore?.modelConfigs?.[modelId] ?? {};
    await patchSettings(
      { modelConfigs: { [modelId]: { ...current, [key]: value } } },
      m.system_catalog_config_saved(),
    );
  }

  /** 模型配置里的 Key（`secret` 字段）：存在凭据库，不在设置里（ADR 048）。这里只记“存了没有”，不回显。 */
  let secretSaved = $state<Record<string, boolean>>({});
  async function refreshSecret(modelId: string) {
    try { secretSaved[modelId] = await modelSecretSaved(modelId); } catch { /* 读不了就当没存 */ }
  }
  $effect(() => {
    for (const model of visibleModels) {
      if (model.configSchema.some((field) => field.kind === "secret")) void refreshSecret(model.id);
    }
  });
  async function saveSecret(modelId: string, key: string, input: HTMLInputElement) {
    const value = input.value.trim();
    if (!value) return;
    input.value = "";
    await saveField(modelId, key, value);
    await refreshSecret(modelId);
  }

  function isStreaming(model: ModelInfo): boolean {
    return model.io === "streaming";
  }

  function sourceLabel(model: ModelInfo): string {
    if (model.backend === "remote") return m.system_catalog_source_online();
    if (model.id.includes("modelscope")) return m.system_models_source_modelscope();
    if (model.recipe === "zipformer-transducer") return m.system_models_source_modelscope_first();
    return m.system_catalog_source_local();
  }
</script>

<div class="space-y-6">
{#if $runtimeStore?.restartNeeded}
  <div class="flex items-center gap-2 rounded-xl border border-warning/40 bg-warning/8 px-3.5 py-2.5">
    <span class="size-2 shrink-0 rounded-full bg-warning"></span>
    <p class="m-0 flex-1 text-xs text-warning">
      {m.system_models_restart_needed()}
    </p>
    <Button size="sm" variant="outline" class="h-7" onclick={() => void startRuntime().then(() => toast(true, m.system_models_restarted())).catch((e) => toast(false, errText(e)))}>{m.system_models_restart_button()}</Button>
  </div>
{/if}

<SettingsSection title={m.system_models_storage_title()}>
  <div class="py-3.5">
    <p class="m-0 text-[13px] font-medium text-foreground">{m.system_models_dir_label()}</p>
    {#if editingDir}
      <div class="mt-2 flex flex-wrap items-center gap-2">
        <input
          bind:value={dirDraft}
          placeholder={m.system_models_dir_placeholder()}
          spellcheck="false"
          class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30"
        />
        <Button variant="outline" size="sm" onclick={() => void onBrowseDir()}>{m.system_models_dir_browse()}</Button>
        <Button size="sm" disabled={migrating || dirDraftError !== null} onclick={() => void onChangeDir()}>
          {migrating ? m.system_models_dir_migrating() : m.system_models_dir_confirm()}
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (editingDir = false)}>{m.system_common_cancel()}</Button>
      </div>
      {#if dirDraftError}
        <p class="mt-2 mb-0 text-xs text-destructive">{dirDraftError}</p>
      {:else}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">
          {m.system_models_dir_edit_hint()}
        </p>
      {/if}
    {:else}
      <div class="mt-2 flex items-center gap-2">
        <span class="min-w-0 flex-1 truncate rounded-lg border border-hairline bg-muted/40 px-2.5 py-1.5 text-xs text-foreground/80">
          {dirInfo?.dir ?? m.system_catalog_loading()}{dirInfo?.isDefault ? m.system_common_default_suffix() : ""}
        </span>
        <Button variant="outline" size="sm" onclick={() => {
          dirDraft = dirInfo && !dirInfo.isDefault ? dirInfo.dir : "";
          editingDir = true;
        }}>{m.system_models_change_button()}</Button>
        <Button variant="outline" size="sm" onclick={() => void onOpenDir()}>{m.system_models_open_dir_button()}</Button>
      </div>
      {#if currentModelsDirError}
        <p class="mt-2 mb-0 text-xs text-destructive">
          {m.system_models_dir_invalid({ error: currentModelsDirError })}
        </p>
      {:else}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">
          {m.system_models_dir_hint()}
        </p>
      {/if}
    {/if}
  </div>
  <div class="py-3.5">
    <p class="m-0 text-[13px] font-medium text-foreground">{m.system_models_history_dir_label()}</p>
    <div class="mt-2 flex items-center gap-2">
      <span class="min-w-0 flex-1 truncate rounded-lg border border-hairline bg-muted/40 px-2.5 py-1.5 text-xs text-foreground/80">
        {historyDirLabel}{historyDirCustom ? "" : m.system_common_default_suffix()}
      </span>
      <Button variant="outline" size="sm" onclick={() => void onChangeHistoryDir()}>{m.system_models_change_button()}</Button>
      {#if historyDirCustom}
        <Button variant="ghost" size="sm" onclick={() => void patchSettings({ history: { dir: "" } }, m.system_models_history_dir_reset_done())}>{m.system_common_restore_default()}</Button>
      {/if}
      <Button variant="outline" size="sm" onclick={() => void onOpenHistoryDir()}>{m.system_models_open_dir_button()}</Button>
    </div>
    <p class="mt-2 mb-0 text-xs text-muted-foreground">
      {m.system_models_history_dir_hint()}
    </p>
  </div>
</SettingsSection>

<div class="space-y-3">
<div id="models-category" class="flex w-fit scroll-mt-16 gap-0.5 rounded-xl bg-muted/70 p-1" role="tablist" aria-label={m.system_models_category_label()}>
  {#each categories as c}
    <button
      type="button"
      role="tab"
      aria-selected={category === c.id}
      data-testid="model-category-{c.id}"
      class="h-7 rounded-lg px-3.5 text-xs font-medium transition-colors {category === c.id
        ? 'bg-background text-foreground shadow-sm'
        : 'text-muted-foreground hover:text-foreground'}"
      onclick={() => (category = c.id)}
    >
      {c.name}
    </button>
  {/each}
</div>

{#if category === "asr" && engines !== null && visibleModels.length > 0}
  <AsrRuntimeCard models={visibleModels} />
{/if}

{#if category === "tts"}
  <!-- 她的声音只在一个地方配（ADR 048）：这里只留一行，点了去声音与弹幕 -->
  <section class="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-hairline bg-card/70 p-4" data-testid="models-tts-moved">
    <p class="m-0 text-xs leading-relaxed text-muted-foreground">{m.system_models_tts_moved()}</p>
    <Button size="sm" variant="outline" onclick={() => openTarget({ page: "assistant", section: "reply", anchor: "settings-reply-output" })}>{m.system_models_tts_open()}</Button>
  </section>
{:else if category !== "asr"}
  {#key category}
    <CatalogModelList {category} />
  {/key}
{:else if engines === null}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt={m.system_models_loading_alt()} class="h-24 w-24 object-contain" />
    <p class="m-0 text-sm text-muted-foreground">{m.system_models_loading()}</p>
  </div>
{:else if visibleModels.length === 0}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt={m.system_models_empty_alt()} class="h-24 w-24 object-contain" />
    <p class="m-0 text-sm text-muted-foreground">{m.system_models_empty()}</p>
  </div>
{:else}
  <section class="rounded-2xl border border-hairline bg-card/70 p-2" data-testid="model-list">
    <p class="m-0 px-2 pt-1.5 pb-2 text-sm font-semibold text-foreground">{m.system_models_list_title()}</p>
    <div class="flex flex-col gap-1" role="radiogroup" aria-label={m.system_models_list_label()}>
      {#each visibleModels as model}
        {@const isActive = isCurrentModel(model.id)}
        {@const streaming = isStreaming(model)}
        {#if model.downloaded}
          <div
            role="radio"
            aria-checked={isActive}
            tabindex="0"
            data-testid="model-row-{model.id}"
            class="flex cursor-pointer items-center gap-2.5 rounded-xl border px-3 py-2.5 transition-colors
              {isActive
              ? 'border-primary/50 bg-primary/8'
              : 'border-transparent hover:bg-muted/60'}"
            onclick={() => !isActive && void onSetActive(model.engineId, model.id)}
            onkeydown={(e) => {
              if (e.key === "Enter" && !isActive) void onSetActive(model.engineId, model.id);
            }}
          >
            <span
              class="flex size-4 shrink-0 items-center justify-center rounded-full border-2 {isActive
                ? 'border-primary'
                : 'border-muted-foreground/40'}"
            >
              {#if isActive}
                <span class="size-1.5 rounded-full bg-primary"></span>
              {/if}
            </span>
            <div class="min-w-0 flex-1">
              <p class="m-0 truncate text-[13px] {isActive ? 'font-medium text-primary' : 'text-foreground'}">
                {model.displayName}
              </p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {formatSize(model.sizeBytes)} · {streaming ? m.system_models_streaming() : m.system_models_non_streaming()} · {sourceLabel(model)}
              </p>
            </div>
            {#if isActive}
              <span class="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">
                {m.system_catalog_active_badge()}
              </span>
            {/if}
            {#if model.backend !== "remote" && confirmingDelete === model.id}
              <Button
                variant="destructive"
                size="xs"
                disabled={deleting !== null}
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  void onDelete(model.id);
                }}
              >
                {deleting === model.id ? m.system_models_deleting() : m.system_catalog_delete_confirm()}
              </Button>
              <Button
                variant="ghost"
                size="xs"
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  confirmingDelete = null;
                }}
              >
                {m.system_common_cancel()}
              </Button>
            {:else if model.backend !== "remote"}
              <Button
                variant="ghost"
                size="xs"
                class="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                title={m.system_models_delete_tooltip()}
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  confirmingDelete = model.id;
                }}
              >
                {m.system_common_delete()}
              </Button>
            {/if}
          </div>
        {:else}
          <div class="rounded-xl px-3 py-2.5" data-testid="model-row-{model.id}">
            <div class="flex items-center gap-2.5">
              <span class="size-4 shrink-0 rounded-full border-2 border-muted-foreground/25"></span>
              <div class="min-w-0 flex-1">
                <p class="m-0 truncate text-[13px] text-muted-foreground">{model.displayName}</p>
                <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground/75">
                  {formatSize(model.sizeBytes)} · {streaming ? m.system_models_streaming() : m.system_models_non_streaming()} · {sourceLabel(model)}
                </p>
              </div>
              <Button
                variant="outline"
                size="xs"
                disabled={downloadingAny}
                data-testid="model-download-{model.id}"
                onclick={() => void onDownload(model.id)}
              >
                {model.id in dlProgress ? m.system_catalog_downloading() : m.system_catalog_download_button()}
              </Button>
            </div>
            {#if model.id in dlProgress}
              <div class="mt-2 flex items-center gap-2 pl-6.5">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                  {#if dlProgress[model.id] !== null}
                    <div
                      class="h-full rounded-full bg-primary transition-[width]"
                      style:width="{dlProgress[model.id]}%"
                    ></div>
                  {:else}
                    <div class="h-full w-1/3 animate-pulse rounded-full bg-primary/70"></div>
                  {/if}
                </div>
                <Button
                  variant="ghost"
                  size="xs"
                  disabled={cancellingId === model.id}
                  onclick={() => void onCancelDownload(model.id)}
                >
                  {cancellingId === model.id ? m.system_models_cancelling() : m.system_common_cancel()}
                </Button>
              </div>
            {/if}
            {#if dlErrors[model.id]}
              <div class="mt-2 ml-6.5 flex items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2">
                <p class="m-0 min-w-0 flex-1 text-[11px] leading-relaxed break-all text-destructive">
                  {m.system_catalog_download_failed({ error: dlErrors[model.id] })}
                </p>
                <Button variant="outline" size="xs" onclick={() => void openManualGuide(model.id, dlErrors[model.id])}>
                  {m.system_catalog_manual_download_button()}
                </Button>
                <Button variant="destructive" size="xs" disabled={downloadingAny} onclick={() => void onDownload(model.id)}>
                  {m.system_models_retry_button()}
                </Button>
              </div>
            {/if}
          </div>
        {/if}
        {#if isActive && model.configSchema.length > 0}
          <div
            class="mx-1 mb-1 rounded-xl border border-hairline bg-muted/40 p-3"
            transition:slide|local={{ duration: 140 }}
          >
            <div class="flex items-center justify-between gap-2">
              <p class="m-0 text-xs font-medium text-foreground">{m.system_catalog_config_title()}</p>
              {#if model.apiKeyUrl}
                <button
                  type="button"
                  class="text-xs font-medium text-primary hover:underline"
                  onclick={() => void openExternal(model.apiKeyUrl!)}
                >
                  {m.system_catalog_get_api_key()}
                </button>
              {/if}
            </div>
            <div class="mt-2 flex flex-col gap-2.5">
              {#each model.configSchema as field}
                <label class="block">
                  <span class="text-xs text-muted-foreground">{field.label}</span>
                  {#if field.kind === "enum"}
                    <div class="mt-1">
                      <SelectField
                        ariaLabel={field.label}
                        value={fieldValue(model.id, field.key, field.default)}
                        options={field.options.map((option) => ({
                          value: option.value,
                          label: option.label,
                        }))}
                        onchange={(next) => void saveField(model.id, field.key, next)}
                      />
                    </div>
                  {:else if field.kind === "connection"}
                    <div class="mt-1">
                      <SelectField
                        ariaLabel={field.label}
                        placeholder={m.system_catalog_connection_placeholder()}
                        value={fieldValue(model.id, field.key, field.default)}
                        options={[
                          { value: "", label: m.system_catalog_connection_placeholder() },
                          ...connections.map((connection) => ({
                            value: connection.id,
                            label: `${connection.displayName}${connection.hasApiKey ? "" : m.system_catalog_connection_no_key_suffix()}`,
                          })),
                        ]}
                        onchange={(next) => void saveField(model.id, field.key, next)}
                      />
                    </div>
                    <p class="mt-1 mb-0 text-[11px] text-muted-foreground/80">
                      {m.system_models_connection_hint()}
                    </p>
                  {:else if field.kind === "secret"}
                    <!-- Key 在凭据库里：只显示存了没有，不回显；填了按回车（或离开输入框）就存 -->
                    <input
                      type="password"
                      autocomplete="off"
                      data-testid="model-secret-{model.id}"
                      class="mt-1 h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30"
                      placeholder={secretSaved[model.id] ? m.system_models_secret_placeholder_saved() : m.system_models_secret_placeholder_new()}
                      onchange={(e) => void saveSecret(model.id, field.key, e.currentTarget)}
                    />
                    {#if secretSaved[model.id]}
                      <p class="mt-1 mb-0 text-[11px] text-success" data-testid="model-secret-saved-{model.id}">{m.system_models_secret_saved()}</p>
                    {/if}
                  {:else}
                    <input
                      class="mt-1 h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30"
                      value={fieldValue(model.id, field.key, field.default)}
                      onchange={(e) =>
                        void saveField(model.id, field.key, (e.target as HTMLInputElement).value)}
                    />
                  {/if}
                </label>
              {/each}
            </div>
          </div>
        {/if}
      {/each}
    </div>
    <p class="mx-2 mt-2 mb-1.5 text-xs text-muted-foreground">
      {m.system_models_list_hint()}
    </p>
  </section>
{/if}
</div>

{#if manualGuide}
  <ManualDownloadDialog
    guide={manualGuide}
    error={manualGuideError}
    onClose={() => {
      manualGuide = null;
      manualGuideError = "";
    }}
    onRetry={() => {
      const id = manualGuide?.id;
      manualGuide = null;
      manualGuideError = "";
      if (id) void onDownload(id);
    }}
    onRecheck={() => void recheckAfterManualInstall()}
  />
{/if}

{#if $settingsStore}
  <SettingsSection title={m.system_models_network_title()} description={m.system_models_network_description()}>
    <div class="grid grid-cols-3 gap-2 py-3.5">
      {#each [
        { id: "auto", name: m.system_models_network_auto(), desc: m.system_models_network_auto_description() },
        { id: "official", name: m.system_models_network_official(), desc: "Hugging Face / GitHub" },
        { id: "mirror", name: m.system_models_network_mirror(), desc: m.system_models_network_mirror_description() },
      ] as const as option}
        {@const selected = $settingsStore.download.source === option.id}
        <button
          type="button"
          aria-pressed={selected}
          class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected
            ? 'border-primary/60 bg-primary/10'
            : 'border-hairline hover:bg-muted/60'}"
          onclick={() =>
            void patchSettings({ download: { source: option.id } }, m.system_models_network_saved({ name: option.name }))}
        >
          <p class="m-0 text-[13px] font-medium {selected ? 'text-primary' : 'text-foreground'}">{option.name}</p>
          <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{option.desc}</p>
        </button>
      {/each}
    </div>
    <div class="py-3.5">
      <label for="download-proxy" class="block text-[13px] font-medium text-foreground">{m.system_models_proxy_label()}</label>
      <div class="mt-2 flex gap-2">
        <input
          id="download-proxy"
          bind:value={downloadProxyDraft}
          class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30"
          placeholder="https://ghfast.top/"
          spellcheck="false"
        />
        <Button variant="outline" size="sm" onclick={() => void saveDownloadProxy()}>{m.system_models_proxy_save()}</Button>
      </div>
    </div>
  </SettingsSection>
{/if}
</div>
