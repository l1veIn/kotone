<script lang="ts">
  import { onMount } from "svelte";
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
  import { spotlight } from "../../../../lib/actions/spotlight";
  import ManualDownloadDialog from "../../../../lib/components/ManualDownloadDialog.svelte";
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
    const ioEngine =
      engineId === "sherpa-onnx-x-asr-zh-en" || engineId === "sherpa-streaming"
        ? "sherpa-streaming"
        : engineId === "sherpa-onnx-sensevoice" ||
            engineId === "sherpa-onnx-funasr-nano" ||
            engineId === "sherpa-offline"
          ? "sherpa-offline"
          : engineId;
    return models.find((m) => m.engineId === ioEngine)?.id === modelId;
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

{#if $runtimeStore?.restartNeeded}
  <div class="flex items-center gap-2 rounded-lg bg-yellow-400/10 px-3 py-2 ring-1 ring-yellow-400/40">
    <span class="h-2 w-2 shrink-0 rounded-full bg-yellow-400"></span>
    <p class="text-[11px] text-yellow-200/90">
      活动模型已变更，运行中的实例仍用旧配置——点标题栏「重启生效」后按新配置运行。
    </p>
  </div>
{/if}

<section class="kotone-panel mt-4 p-4">
  <h2 class="text-sm font-semibold text-kotone-cyan/90">模型存储位置</h2>
  {#if editingDir}
    <div class="mt-3 flex items-center gap-2">
      <input
        bind:value={dirDraft}
        placeholder="留空 = 默认 ~/.kotone/models"
        spellcheck="false"
        class="min-w-0 flex-1 rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
      />
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/75 ring-1 ring-white/15 transition hover:bg-white/20"
        onclick={() => void onBrowseDir()}
      >
        浏览…
      </button>
      <button
        class="shrink-0 rounded-lg bg-kotone-cyan px-3 py-1.5 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={migrating || dirDraftError !== null}
        onclick={() => void onChangeDir()}
      >
        {migrating ? "迁移中…" : "确认并迁移"}
      </button>
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/70 transition hover:bg-white/20"
        onclick={() => (editingDir = false)}
      >
        取消
      </button>
    </div>
    {#if dirDraftError}
      <p class="mt-2 text-[11px] text-kotone-pink">{dirDraftError}</p>
    {:else}
      <p class="mt-2 text-[11px] text-white/40">
        仅支持纯英文路径；已下载的模型会移动到新目录（迁移失败的条目需重新下载）。
      </p>
    {/if}
  {:else}
    <div class="mt-3 flex items-center gap-2">
      <span class="min-w-0 flex-1 truncate rounded-lg bg-white/5 px-2.5 py-1.5 text-xs text-white/70 ring-1 ring-white/10">
        {dirInfo?.dir ?? "读取中…"}{dirInfo?.isDefault ? "（默认）" : ""}
      </span>
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20 active:scale-95"
        onclick={() => {
          dirDraft = dirInfo && !dirInfo.isDefault ? dirInfo.dir : "";
          editingDir = true;
        }}
      >
        更改
      </button>
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20 active:scale-95"
        onclick={() => void onOpenDir()}
      >
        打开目录
      </button>
    </div>
    {#if currentModelsDirError}
      <p class="mt-2 text-[11px] text-kotone-pink">
        {currentModelsDirError}。请点击“更改”选择新目录。
      </p>
    {:else}
      <p class="mt-2 text-[11px] text-white/40">
        模型路径需要使用纯英文，例如 D:\KotoneModels。
      </p>
    {/if}
  {/if}
</section>

<section class="kotone-panel mt-3 p-4">
  <h2 class="text-sm font-semibold text-kotone-cyan/90">历史记录位置</h2>
  <div class="mt-3 flex items-center gap-2">
    <span class="min-w-0 flex-1 truncate rounded-lg bg-white/5 px-2.5 py-1.5 text-xs text-white/70 ring-1 ring-white/10">
      {historyDirLabel}{historyDirCustom ? "" : "（默认）"}
    </span>
    <button
      class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20"
      onclick={() => void onChangeHistoryDir()}
    >
      更改
    </button>
    {#if historyDirCustom}
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/70 ring-1 ring-white/15 transition hover:bg-white/20"
        onclick={() => void patchSettings({ history: { dir: "" } }, "已恢复默认历史目录")}
      >
        恢复默认
      </button>
    {/if}
    <button
      class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20"
      onclick={() => void onOpenHistoryDir()}
    >
      打开目录
    </button>
  </div>
  <p class="mt-2 text-[11px] text-white/40">
    历史记录与回放音频保存位置；新记录写入新目录，旧目录内容不自动迁移。
  </p>
</section>

{#if engines === null}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt="加载中" class="h-24 w-24 object-contain" />
    <p class="text-sm text-white/50">Kotone 正在清点模型…</p>
  </div>
{:else if visibleModels.length === 0}
  <div class="mt-10 flex flex-col items-center gap-3">
    <img src={stickerCurious} alt="空空如也" class="h-24 w-24 object-contain" />
    <p class="text-sm text-white/50">还没有可用模型</p>
  </div>
{:else}
  <section class="kotone-panel mt-3 p-4" data-testid="model-list">
    <div class="flex items-start justify-between gap-3">
      <div>
        <h2 class="text-sm font-semibold text-kotone-cyan/90">模型切换</h2>
      </div>
    </div>
    <div class="mt-3 flex flex-col gap-1.5" role="radiogroup" aria-label="sherpa-onnx 模型选择">
      {#each visibleModels as m}
        {@const isActive = isCurrentModel(m.id)}
        {@const streaming = isStreaming(m)}
        {#if m.downloaded}
          <div
            use:spotlight
            role="radio"
            aria-checked={isActive}
            tabindex="0"
            class="kotone-spotlight flex cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 ring-1 transition
              {isActive
              ? 'bg-kotone-cyan/10 ring-kotone-cyan/60 shadow-glow-cyan'
              : 'bg-white/4 ring-white/10 hover:bg-white/8 hover:shadow-[0_0_12px_rgba(0,229,255,0.14)] hover:ring-white/25'}"
            onclick={() => !isActive && void onSetActive(m.engineId, m.id)}
            onkeydown={(e) => {
              if (e.key === "Enter" && !isActive) void onSetActive(m.engineId, m.id);
            }}
          >
            <span
              class="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full ring-2 {isActive
                ? 'ring-kotone-cyan'
                : 'ring-white/30'}"
            >
              {#if isActive}
                <span class="h-1.5 w-1.5 rounded-full bg-kotone-cyan shadow-glow-cyan"></span>
              {/if}
            </span>
            <div class="min-w-0 flex-1">
              <p class="truncate text-[12px] {isActive ? 'font-semibold text-kotone-cyan' : 'text-white/80'}">
                {m.displayName}
              </p>
              <p class="mt-0.5 text-[10px] text-white/35">
                {formatSize(m.sizeBytes)} · {streaming ? "流式" : "非流式"} · {sourceLabel(m)}
              </p>
            </div>
            {#if isActive}
              <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] font-semibold text-kotone-cyan">
                使用中
              </span>
            {/if}
            {#if m.backend !== "remote" && confirmingDelete === m.id}
              <button
                class="shrink-0 rounded-lg bg-kotone-pink px-2.5 py-1 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95 disabled:opacity-50"
                disabled={deleting !== null}
                onclick={(e) => {
                  e.stopPropagation();
                  void onDelete(m.id);
                }}
              >
                {deleting === m.id ? "删除中…" : "确认删除"}
              </button>
              <button
                class="shrink-0 rounded-lg bg-white/10 px-2 py-1 text-[11px] text-white/60 transition hover:bg-white/20"
                onclick={(e) => {
                  e.stopPropagation();
                  confirmingDelete = null;
                }}
              >
                取消
              </button>
            {:else if m.backend !== "remote"}
              <button
                class="shrink-0 rounded-lg bg-white/8 px-2 py-1 text-[11px] text-white/45 ring-1 ring-white/12 transition hover:bg-kotone-pink/20 hover:text-kotone-pink active:scale-95"
                title="删除模型文件"
                onclick={(e) => {
                  e.stopPropagation();
                  confirmingDelete = m.id;
                }}
              >
                删除
              </button>
            {/if}
          </div>
        {:else}
          <div class="rounded-lg bg-white/3 px-3 py-2 ring-1 ring-white/8">
            <div class="flex items-center gap-2.5">
              <span class="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full ring-2 ring-white/15"></span>
              <div class="min-w-0 flex-1">
                <p class="truncate text-[12px] text-white/45">{m.displayName}</p>
                <p class="mt-0.5 text-[10px] text-white/30">
                  {formatSize(m.sizeBytes)} · {streaming ? "流式" : "非流式"} · {sourceLabel(m)}
                </p>
              </div>
              <button
                class="shrink-0 rounded-lg bg-kotone-violet/25 px-2.5 py-1 text-[11px] font-semibold text-kotone-violet ring-1 ring-kotone-violet/40 transition hover:bg-kotone-violet/35 hover:shadow-[0_0_12px_rgba(123,47,255,0.45)] active:scale-95 disabled:opacity-50"
                disabled={downloadingAny}
                onclick={() => void onDownload(m.id)}
              >
                {m.id in dlProgress ? "下载中…" : "下载"}
              </button>
            </div>
            {#if m.id in dlProgress}
              <div class="mt-2 flex items-center gap-2">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-white/10">
                  {#if dlProgress[m.id] !== null}
                    <div
                      class="h-full rounded-full bg-kotone-violet transition-[width]"
                      style:width="{dlProgress[m.id]}%"
                    ></div>
                  {:else}
                    <div class="h-full w-1/3 animate-pulse rounded-full bg-kotone-violet/70"></div>
                  {/if}
                </div>
                <button
                  class="shrink-0 rounded-lg bg-white/10 px-2.5 py-1 text-[11px] text-white/70 ring-1 ring-white/15 transition hover:bg-white/20 disabled:opacity-50"
                  disabled={cancellingId === m.id}
                  onclick={() => void onCancelDownload(m.id)}
                >
                  {cancellingId === m.id ? "正在取消…" : "取消"}
                </button>
              </div>
            {/if}
            {#if dlErrors[m.id]}
              <div class="mt-2 flex items-start gap-2 rounded-lg bg-kotone-pink/10 px-2.5 py-2 ring-1 ring-kotone-pink/35">
                <p class="min-w-0 flex-1 text-[10px] leading-relaxed break-all text-kotone-pink">
                  下载失败：{dlErrors[m.id]}
                </p>
                <button
                  class="shrink-0 rounded bg-white/10 px-2 py-1 text-[10px] font-semibold text-white/75 ring-1 ring-white/15 hover:bg-white/20"
                  onclick={() => void openManualGuide(m.id, dlErrors[m.id])}
                >
                  手动下载
                </button>
                <button
                  class="shrink-0 rounded bg-kotone-pink/80 px-2 py-1 text-[10px] font-semibold text-white hover:brightness-110 disabled:opacity-50"
                  disabled={downloadingAny}
                  onclick={() => void onDownload(m.id)}
                >
                  重试
                </button>
              </div>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    {#if $settingsStore}
      {@const active = visibleModels.find((item) => isCurrentModel(item.id))}
      {#if active && active.configSchema.length > 0}
        <div class="mt-3 rounded-lg bg-white/4 p-3 ring-1 ring-white/10">
          <p class="text-[11px] font-semibold text-white/70">模型配置</p>
          <div class="mt-2 flex flex-col gap-2">
            {#each active.configSchema as field}
              <label class="block">
                <span class="text-[11px] text-white/50">{field.label}</span>
                {#if field.kind === "enum"}
                  <select
                    class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
                    value={fieldValue(active.id, field.key, field.default)}
                    onchange={(e) =>
                      void saveField(active.id, field.key, (e.target as HTMLSelectElement).value)}
                  >
                    {#each field.options as option}
                      <option value={option.value}>{option.label}</option>
                    {/each}
                  </select>
                {:else if field.kind === "connection"}
                  <select
                    class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
                    value={fieldValue(active.id, field.key, field.default)}
                    onchange={(e) =>
                      void saveField(active.id, field.key, (e.target as HTMLSelectElement).value)}
                  >
                    <option value="">请选择 API 连接</option>
                    {#each connections as connection}
                      <option value={connection.id}>
                        {connection.displayName}{connection.hasApiKey ? "" : "（未填密钥）"}
                      </option>
                    {/each}
                  </select>
                  <p class="mt-1 text-[10px] text-white/35">
                    地址和 API key 在「高级 → API 连接」里保存，不会写入配置文件。
                  </p>
                {:else}
                  <input
                    class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60"
                    value={fieldValue(active.id, field.key, field.default)}
                    onchange={(e) =>
                      void saveField(active.id, field.key, (e.target as HTMLInputElement).value)}
                  />
                {/if}
              </label>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
    <p class="mt-3 text-[11px] text-white/40">
      本机模型未下载时「启动」会提示缺失项；切换后如已「启动」，需点标题栏「重启生效」。
    </p>
  </section>
{/if}

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
  <section class="kotone-panel mt-4 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">下载网络</h2>
    <p class="mt-1 text-[11px] text-white/45">默认优先使用魔搭国内镜像，失败后自动回退官方源。</p>
    <div class="mt-3 grid grid-cols-3 gap-2">
      {#each [
        { id: "auto", name: "自动", desc: "魔搭优先，回退官方" },
        { id: "official", name: "仅官方", desc: "Hugging Face / GitHub" },
        { id: "mirror", name: "仅镜像", desc: "魔搭 / HF 镜像" },
      ] as option}
        {@const selected = $settingsStore.download.source === option.id}
        <button
          class="rounded-lg px-2.5 py-2 text-left ring-1 transition {selected
            ? 'bg-kotone-cyan/12 ring-kotone-cyan/60'
            : 'bg-white/5 ring-white/10 hover:bg-white/10'}"
          onclick={() =>
            void patchSettings({ download: { source: option.id } }, `下载源已切换：${option.name}`)}
        >
          <p class="text-xs font-semibold {selected ? 'text-kotone-cyan' : ''}">{option.name}</p>
          <p class="mt-0.5 text-[10px] leading-relaxed text-white/40">{option.desc}</p>
        </button>
      {/each}
    </div>
    <label class="mt-3 block">
      <span class="text-[11px] text-white/50">GitHub 备用代理（可选）</span>
      <div class="mt-1 flex gap-2">
        <input
          bind:value={downloadProxyDraft}
          class="min-w-0 flex-1 rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60"
          placeholder="https://ghfast.top/"
          spellcheck="false"
        />
        <button
          class="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/75 hover:bg-white/20"
          onclick={() => void saveDownloadProxy()}
        >
          保存
        </button>
      </div>
    </label>
  </section>
{/if}
