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
    type DownloadProgress,
    type EngineInfo,
    type ConnectionInfo,
    type ModelInfo,
    type ModelInstallGuide,
    type ModelsDirInfo,
  } from "../../../../lib/ipc";
  import { settingsStore, patchSettings, toast, toastInfo, toastWarn, errText } from "../../../../lib/stores/ui";
  import { runtimeStore } from "../../../../lib/stores/runtime";
  import ManualDownloadDialog from "../../../../lib/components/ManualDownloadDialog.svelte";
  import CatalogModelList from "./CatalogModelList.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { Button } from "$lib/components/ui/button";
  import stickerCurious from "../../../../assets/brand/stickers/curious.webp";

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
  let category = $state<"asr" | "ocr" | "tts" | "search">("asr");
  const categories = [
    { id: "asr", name: "语音识别" },
    { id: "ocr", name: "文字识别" },
    { id: "tts", name: "语音合成" },
    { id: "search", name: "联网搜索" },
  ] as const;

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);
  const dirDraftError = $derived(modelsDirPathError(dirDraft.trim()));
  const currentModelsDirError = $derived(dirInfo ? modelsDirPathError(dirInfo.dir) : null);
  const historyDirLabel = $derived($settingsStore?.history.dir.trim() || "~/.kotone/history");
  const historyDirCustom = $derived(Boolean($settingsStore?.history.dir.trim()));

  const visibleModels = $derived(models.filter((m) => m.engineId !== "mock-stream"));

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
        toast(false, `加载引擎列表失败：${errText(e)}`);
        return null;
      }),
      listModels().catch((e) => {
        console.error("[engine-page] list_models 失败：", e);
        toast(false, `加载模型清单失败：${errText(e)}`);
        return null;
      }),
      getModelsDir().catch((e) => {
        console.error("[engine-page] get_models_dir 失败：", e);
        toast(false, `读取模型目录失败：${errText(e)}`);
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
        toastWarn(`已切换，但 ${report.failed.length} 项迁移失败（需重新下载）：${report.failed.join(", ")}`);
      } else {
        toast(true, report.moved.length > 0 ? `模型目录已切换，迁移 ${report.moved.length} 项` : "模型目录已切换");
      }
      editingDir = false;
      await reload();
    } catch (e) {
      toast(false, `切换模型目录失败：${errText(e)}`);
    } finally {
      migrating = false;
    }
  }

  async function onOpenDir() {
    try {
      await openModelsDir();
    } catch (e) {
      toast(false, `打开目录失败：${errText(e)}`);
    }
  }

  async function onBrowseDir() {
    const sel = await openDialog({ directory: true, title: "选择模型存储位置" }).catch(() => null);
    if (!sel) return;
    const pathError = modelsDirPathError(sel);
    if (pathError) {
      toast(false, pathError);
      return;
    }
    dirDraft = sel;
  }

  async function onChangeHistoryDir() {
    const sel = await openDialog({ directory: true, title: "选择历史记录位置" }).catch(() => null);
    if (!sel) return;
    await patchSettings({ history: { dir: sel } }, "历史目录已切换");
  }

  async function onOpenHistoryDir() {
    try {
      await openHistoryDir();
    } catch (e) {
      toast(false, `打开目录失败：${errText(e)}`);
    }
  }

  async function onDownload(id: string) {
    if (downloadingAny) return;
    delete dlErrors[id];
    dlProgress[id] = null;
    try {
      await downloadModel(id);
      models = models.map((m) => (m.id === id ? { ...m, downloaded: true } : m));
      toast(true, `下载完成：${id}`);
      await reload();
    } catch (e) {
      if (isDownloadCancelled(e)) {
        toastInfo("已取消下载，可随时继续");
      } else {
        dlErrors[id] = errText(e);
        toast(false, "下载失败，可按指引手动下载或调整下载源后重试");
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
      toast(false, `取消失败：${errText(e)}`);
    }
  }

  async function openManualGuide(id: string, error = "") {
    try {
      manualGuide = await getModelInstallGuide(id);
      manualGuideError = error;
    } catch (e) {
      toast(false, `无法加载手动安装指引：${errText(e)}`);
    }
  }

  async function recheckAfterManualInstall() {
    const id = manualGuide?.id;
    await reload();
    if (id && models.some((m) => m.id === id && m.downloaded)) {
      delete dlErrors[id];
      manualGuide = null;
      manualGuideError = "";
      toast(true, "已检测到模型文件");
    } else {
      toastWarn("还没检测到完整模型，请确认文件名和目录后重试");
    }
  }

  async function onDelete(id: string) {
    if (deleting) return;
    deleting = id;
    confirmingDelete = null;
    try {
      const outcome = await deleteModel(id);
      models = models.map((m) => (m.id === id ? { ...m, downloaded: false } : m));
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
        toast(true, `已删除：${id}（活动模型已回退默认）`);
      } else {
        toast(true, `已删除：${id}`);
      }
      await reload();
    } catch (e) {
      toast(false, `删除失败：${errText(e)}`);
    } finally {
      deleting = null;
    }
  }

  async function onSetActive(engineId: string, modelId: string) {
    try {
      await setActiveModel(engineId, modelId);
      settingsStore.update((s) => {
        if (!s) return s;
        const next = structuredClone(s);
        next.activeModelId = modelId;
        next.sttEngine = engineId;
        ((next.engineOptions[engineId] ??= {}) as Record<string, unknown>).model = modelId;
        return next;
      });
      if ($runtimeStore?.phase === "running") {
        toastWarn(`已切换模型，重启后生效`);
      } else {
        toast(true, "已切换模型");
      }
    } catch (e) {
      toast(false, `切换模型失败：${errText(e)}`);
    }
  }

  async function saveDownloadProxy() {
    await patchSettings(
      { download: { ghProxy: downloadProxyDraft.trim() } },
      "下载代理地址已保存",
    );
  }

  function formatSize(bytes: number): string {
    if (bytes <= 0) return "无需下载";
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
    const defaultModel = models.find((m) => m.engineId === engineId);
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
      "模型配置已保存",
    );
  }

  function isStreaming(model: ModelInfo): boolean {
    return model.io === "streaming";
  }

  function sourceLabel(model: ModelInfo): string {
    if (model.backend === "remote") return "在线";
    if (model.id.includes("modelscope")) return "魔搭";
    if (model.recipe === "zipformer-transducer") return "魔搭优先";
    return "本机";
  }
</script>

<div class="space-y-6">
{#if $runtimeStore?.restartNeeded}
  <div class="flex items-center gap-2 rounded-xl border border-warning/40 bg-warning/8 px-3.5 py-2.5">
    <span class="size-2 shrink-0 rounded-full bg-warning"></span>
    <p class="m-0 text-xs text-warning">
      活动模型已变更，运行中的实例仍用旧配置——点标题栏「重启生效」后按新配置运行。
    </p>
  </div>
{/if}

<SettingsSection title="存储位置">
  <div class="py-3.5">
    <p class="m-0 text-[13px] font-medium text-foreground">模型存储位置</p>
    {#if editingDir}
      <div class="mt-2 flex flex-wrap items-center gap-2">
        <input
          bind:value={dirDraft}
          placeholder="留空 = 默认 ~/.kotone/models"
          spellcheck="false"
          class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30"
        />
        <Button variant="outline" size="sm" onclick={() => void onBrowseDir()}>浏览…</Button>
        <Button size="sm" disabled={migrating || dirDraftError !== null} onclick={() => void onChangeDir()}>
          {migrating ? "迁移中…" : "确认并迁移"}
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (editingDir = false)}>取消</Button>
      </div>
      {#if dirDraftError}
        <p class="mt-2 mb-0 text-xs text-destructive">{dirDraftError}</p>
      {:else}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">
          仅支持纯英文路径；已下载的模型会移动到新目录（迁移失败的条目需重新下载）。
        </p>
      {/if}
    {:else}
      <div class="mt-2 flex items-center gap-2">
        <span class="min-w-0 flex-1 truncate rounded-lg border border-hairline bg-muted/40 px-2.5 py-1.5 text-xs text-foreground/80">
          {dirInfo?.dir ?? "读取中…"}{dirInfo?.isDefault ? "（默认）" : ""}
        </span>
        <Button variant="outline" size="sm" onclick={() => {
          dirDraft = dirInfo && !dirInfo.isDefault ? dirInfo.dir : "";
          editingDir = true;
        }}>更改</Button>
        <Button variant="outline" size="sm" onclick={() => void onOpenDir()}>打开目录</Button>
      </div>
      {#if currentModelsDirError}
        <p class="mt-2 mb-0 text-xs text-destructive">
          {currentModelsDirError}。请点击“更改”选择新目录。
        </p>
      {:else}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">
          模型路径需要使用纯英文，例如 D:\KotoneModels。
        </p>
      {/if}
    {/if}
  </div>
  <div class="py-3.5">
    <p class="m-0 text-[13px] font-medium text-foreground">历史记录位置</p>
    <div class="mt-2 flex items-center gap-2">
      <span class="min-w-0 flex-1 truncate rounded-lg border border-hairline bg-muted/40 px-2.5 py-1.5 text-xs text-foreground/80">
        {historyDirLabel}{historyDirCustom ? "" : "（默认）"}
      </span>
      <Button variant="outline" size="sm" onclick={() => void onChangeHistoryDir()}>更改</Button>
      {#if historyDirCustom}
        <Button variant="ghost" size="sm" onclick={() => void patchSettings({ history: { dir: "" } }, "已恢复默认历史目录")}>恢复默认</Button>
      {/if}
      <Button variant="outline" size="sm" onclick={() => void onOpenHistoryDir()}>打开目录</Button>
    </div>
    <p class="mt-2 mb-0 text-xs text-muted-foreground">
      历史记录与回放音频保存位置；新记录写入新目录，旧目录内容不自动迁移。
    </p>
  </div>
</SettingsSection>

<div class="space-y-3">
<div class="flex w-fit gap-0.5 rounded-xl bg-muted/70 p-1" role="tablist" aria-label="模型类别">
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

{#if category !== "asr"}
  {#key category}
    <CatalogModelList {category} />
  {/key}
{:else if engines === null}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt="加载中" class="h-24 w-24 object-contain" />
    <p class="m-0 text-sm text-muted-foreground">Kotone 正在清点模型…</p>
  </div>
{:else if visibleModels.length === 0}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt="空空如也" class="h-24 w-24 object-contain" />
    <p class="m-0 text-sm text-muted-foreground">还没有可用模型</p>
  </div>
{:else}
  <section class="rounded-2xl border border-hairline bg-card/70 p-2" data-testid="model-list">
    <p class="m-0 px-2 pt-1.5 pb-2 text-sm font-semibold text-foreground">模型切换</p>
    <div class="flex flex-col gap-1" role="radiogroup" aria-label="sherpa-onnx 模型选择">
      {#each visibleModels as m}
        {@const isActive = isCurrentModel(m.id)}
        {@const streaming = isStreaming(m)}
        {#if m.downloaded}
          <div
            role="radio"
            aria-checked={isActive}
            tabindex="0"
            class="flex cursor-pointer items-center gap-2.5 rounded-xl border px-3 py-2.5 transition-colors
              {isActive
              ? 'border-primary/50 bg-primary/8'
              : 'border-transparent hover:bg-muted/60'}"
            onclick={() => !isActive && void onSetActive(m.engineId, m.id)}
            onkeydown={(e) => {
              if (e.key === "Enter" && !isActive) void onSetActive(m.engineId, m.id);
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
                {m.displayName}
              </p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {formatSize(m.sizeBytes)} · {streaming ? "流式" : "非流式"} · {sourceLabel(m)}
              </p>
            </div>
            {#if isActive}
              <span class="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">
                使用中
              </span>
            {/if}
            {#if m.backend !== "remote" && confirmingDelete === m.id}
              <Button
                variant="destructive"
                size="xs"
                disabled={deleting !== null}
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  void onDelete(m.id);
                }}
              >
                {deleting === m.id ? "删除中…" : "确认删除"}
              </Button>
              <Button
                variant="ghost"
                size="xs"
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  confirmingDelete = null;
                }}
              >
                取消
              </Button>
            {:else if m.backend !== "remote"}
              <Button
                variant="ghost"
                size="xs"
                class="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                title="删除模型文件"
                onclick={(e: MouseEvent) => {
                  e.stopPropagation();
                  confirmingDelete = m.id;
                }}
              >
                删除
              </Button>
            {/if}
          </div>
        {:else}
          <div class="rounded-xl px-3 py-2.5">
            <div class="flex items-center gap-2.5">
              <span class="size-4 shrink-0 rounded-full border-2 border-muted-foreground/25"></span>
              <div class="min-w-0 flex-1">
                <p class="m-0 truncate text-[13px] text-muted-foreground">{m.displayName}</p>
                <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground/75">
                  {formatSize(m.sizeBytes)} · {streaming ? "流式" : "非流式"} · {sourceLabel(m)}
                </p>
              </div>
              <Button
                variant="outline"
                size="xs"
                disabled={downloadingAny}
                onclick={() => void onDownload(m.id)}
              >
                {m.id in dlProgress ? "下载中…" : "下载"}
              </Button>
            </div>
            {#if m.id in dlProgress}
              <div class="mt-2 flex items-center gap-2 pl-6.5">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                  {#if dlProgress[m.id] !== null}
                    <div
                      class="h-full rounded-full bg-primary transition-[width]"
                      style:width="{dlProgress[m.id]}%"
                    ></div>
                  {:else}
                    <div class="h-full w-1/3 animate-pulse rounded-full bg-primary/70"></div>
                  {/if}
                </div>
                <Button
                  variant="ghost"
                  size="xs"
                  disabled={cancellingId === m.id}
                  onclick={() => void onCancelDownload(m.id)}
                >
                  {cancellingId === m.id ? "正在取消…" : "取消"}
                </Button>
              </div>
            {/if}
            {#if dlErrors[m.id]}
              <div class="mt-2 ml-6.5 flex items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2">
                <p class="m-0 min-w-0 flex-1 text-[11px] leading-relaxed break-all text-destructive">
                  下载失败：{dlErrors[m.id]}
                </p>
                <Button variant="outline" size="xs" onclick={() => void openManualGuide(m.id, dlErrors[m.id])}>
                  手动下载
                </Button>
                <Button variant="destructive" size="xs" disabled={downloadingAny} onclick={() => void onDownload(m.id)}>
                  重试
                </Button>
              </div>
            {/if}
          </div>
        {/if}
        {#if isActive && m.configSchema.length > 0}
          <div
            class="mx-1 mb-1 rounded-xl border border-hairline bg-muted/40 p-3"
            transition:slide|local={{ duration: 140 }}
          >
            <div class="flex items-center justify-between gap-2">
              <p class="m-0 text-xs font-medium text-foreground">模型配置</p>
              {#if m.apiKeyUrl}
                <button
                  type="button"
                  class="text-xs font-medium text-primary hover:underline"
                  onclick={() => void openExternal(m.apiKeyUrl!)}
                >
                  获取 API Key
                </button>
              {/if}
            </div>
            <div class="mt-2 flex flex-col gap-2.5">
              {#each m.configSchema as field}
                <label class="block">
                  <span class="text-xs text-muted-foreground">{field.label}</span>
                  {#if field.kind === "enum"}
                    <div class="mt-1">
                      <SelectField
                        ariaLabel={field.label}
                        value={fieldValue(m.id, field.key, field.default)}
                        options={field.options.map((option) => ({
                          value: option.value,
                          label: option.label,
                        }))}
                        onchange={(next) => void saveField(m.id, field.key, next)}
                      />
                    </div>
                  {:else if field.kind === "connection"}
                    <div class="mt-1">
                      <SelectField
                        ariaLabel={field.label}
                        placeholder="请选择 API 连接"
                        value={fieldValue(m.id, field.key, field.default)}
                        options={[
                          { value: "", label: "请选择 API 连接" },
                          ...connections.map((connection) => ({
                            value: connection.id,
                            label: `${connection.displayName}${connection.hasApiKey ? "" : "（未填密钥）"}`,
                          })),
                        ]}
                        onchange={(next) => void saveField(m.id, field.key, next)}
                      />
                    </div>
                    <p class="mt-1 mb-0 text-[11px] text-muted-foreground/80">
                      地址和 API key 在「高级 → API 连接」里保存，不会写入配置文件。
                    </p>
                  {:else}
                    <input
                      class="mt-1 h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30"
                      value={fieldValue(m.id, field.key, field.default)}
                      onchange={(e) =>
                        void saveField(m.id, field.key, (e.target as HTMLInputElement).value)}
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
      本机模型未下载时「启动」会提示缺失项；切换后如已「启动」，需点标题栏「重启生效」。
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
  <SettingsSection title="下载网络" description="默认优先使用魔搭国内镜像，失败后自动回退官方源。">
    <div class="grid grid-cols-3 gap-2 py-3.5">
      {#each [
        { id: "auto", name: "自动", desc: "魔搭优先，回退官方" },
        { id: "official", name: "仅官方", desc: "Hugging Face / GitHub" },
        { id: "mirror", name: "仅镜像", desc: "魔搭 / HF 镜像" },
      ] as const as option}
        {@const selected = $settingsStore.download.source === option.id}
        <button
          type="button"
          aria-pressed={selected}
          class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected
            ? 'border-primary/60 bg-primary/10'
            : 'border-hairline hover:bg-muted/60'}"
          onclick={() =>
            void patchSettings({ download: { source: option.id } }, `下载源已切换：${option.name}`)}
        >
          <p class="m-0 text-[13px] font-medium {selected ? 'text-primary' : 'text-foreground'}">{option.name}</p>
          <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{option.desc}</p>
        </button>
      {/each}
    </div>
    <div class="py-3.5">
      <label for="download-proxy" class="block text-[13px] font-medium text-foreground">GitHub 备用代理（可选）</label>
      <div class="mt-2 flex gap-2">
        <input
          id="download-proxy"
          bind:value={downloadProxyDraft}
          class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30"
          placeholder="https://ghfast.top/"
          spellcheck="false"
        />
        <Button variant="outline" size="sm" onclick={() => void saveDownloadProxy()}>保存</Button>
      </div>
    </div>
  </SettingsSection>
{/if}
</div>
