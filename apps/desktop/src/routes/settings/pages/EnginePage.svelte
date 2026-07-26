<script lang="ts">
  /*
   * 引擎与模型页（模型管理升级版）：
   * - 顶部：模型存储位置（当前路径 + 更改[迁移] + 打开目录）；
   * - restartNeeded 黄条（与标题栏「重启生效」联动）；
   * - 按引擎分组的完整模型清单：名称/大小/流式标签/已下载/active 标记 +
   *   下载（进度条）/ 删除（二次确认）/ 设为 active；
   * - VAD 组件单列一组（不可设为 active）；whisper-cli 运行时归 whisper 组。
   */
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    listSttEngines,
    setSttEngine,
    listModels,
    downloadModel,
    setActiveModel,
    deleteModel,
    getModelsDir,
    setModelsDir,
    openModelsDir,
    isTauri,
    type DownloadProgress,
    type EngineInfo,
    type ModelInfo,
    type ModelsDirInfo,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import { runtimeStore } from "../../../lib/stores/runtime";
  import stickerCurious from "../../../assets/brand/stickers/curious.png";

  let engines = $state<EngineInfo[] | null>(null);
  let models = $state<ModelInfo[]>([]);
  let dirInfo = $state<ModelsDirInfo | null>(null);
  /** 下载中模型 id → 进度百分比（null = 连接中） */
  let dlProgress = $state<Record<string, number | null>>({});
  /** 二次确认删除中的模型 id */
  let confirmingDelete = $state<string | null>(null);
  let deleting = $state<string | null>(null);
  /** 目录更改模式：输入草稿 */
  let editingDir = $state(false);
  let dirDraft = $state("");
  let migrating = $state(false);

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);

  onMount(() => {
    let un: (() => void) | undefined;
    void (async () => {
      await reload();
      // 下载进度事件（Tauri 环境；mock 无事件，downloadModel 直接置完成）
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
    try {
      [engines, models, dirInfo] = await Promise.all([
        listSttEngines(),
        listModels(),
        getModelsDir(),
      ]);
    } catch (e) {
      toast(false, `加载引擎信息失败：${errText(e)}`);
      engines = [];
    }
  }

  // ---------- 模型存储位置 ----------

  async function onChangeDir() {
    const dir = dirDraft.trim();
    editingDir = false;
    if (migrating || dir === (dirInfo?.isDefault ? "" : dirInfo?.dir)) return;
    migrating = true;
    try {
      const report = await setModelsDir(dir);
      if (report.failed.length > 0) {
        toast(false, `已切换，但 ${report.failed.length} 项迁移失败（需重新下载）：${report.failed.join(", ")}`);
      } else {
        toast(true, report.moved.length > 0 ? `模型目录已切换，迁移 ${report.moved.length} 项` : "模型目录已切换");
      }
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

  // ---------- 引擎 / 模型操作 ----------

  async function onEngineSwitch(id: string) {
    try {
      await setSttEngine(id);
      settingsStore.update((s) => (s ? { ...s, sttEngine: id } : s));
      const name = engines?.find((en) => en.id === id)?.displayName ?? id;
      toast(true, `已切换到引擎：${name}${$runtimeStore?.phase === "running" ? "（需重启生效）" : ""}`);
    } catch (e) {
      toast(false, `切换引擎失败：${errText(e)}`);
    }
  }

  async function onDownload(id: string) {
    if (downloadingAny) return;
    dlProgress[id] = null;
    try {
      await downloadModel(id);
      models = models.map((m) => (m.id === id ? { ...m, downloaded: true } : m));
      toast(true, `下载完成：${id}`);
      await reload();
    } catch (e) {
      toast(false, `下载失败：${errText(e)}`);
    } finally {
      delete dlProgress[id];
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
        // active 标记已回退默认：同步 settingsStore 的 engineOptions
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
        ((next.engineOptions[engineId] ??= {}) as Record<string, unknown>).model = modelId;
        return next;
      });
      toast(true, `活动模型已切换：${modelId}${$runtimeStore?.phase === "running" ? "（需重启生效）" : ""}`);
    } catch (e) {
      toast(false, `切换活动模型失败：${errText(e)}`);
    }
  }

  // ---------- 展示辅助 ----------

  function engineTags(en: EngineInfo): { text: string; cls: string }[] {
    const tags: { text: string; cls: string }[] = [];
    if (en.capabilities.streaming) tags.push({ text: "流式", cls: "bg-kotone-cyan/15 text-kotone-cyan" });
    if (en.capabilities.offline) tags.push({ text: "离线", cls: "bg-kotone-violet/15 text-kotone-violet" });
    if (en.capabilities.hotwords) tags.push({ text: "热词", cls: "bg-white/10 text-white/70" });
    if (!en.isReady) tags.push({ text: "未就绪", cls: "bg-kotone-pink/15 text-kotone-pink" });
    return tags;
  }

  function formatSize(bytes: number): string {
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  /** 引擎当前活动模型 id（engineOptions 未配置按引擎默认） */
  function activeModelOf(engineId: string): string {
    const configured = ($settingsStore?.engineOptions?.[engineId] as Record<string, unknown> | undefined)
      ?.model as string | undefined;
    if (configured) return configured;
    return engineId === "sherpa-onnx-zipformer-zh" ? "zipformer-bilingual-zh-en-2023-02-20" : "ggml-small";
  }

  function modelsOf(engineId: string): ModelInfo[] {
    return models.filter((m) => m.engineId === engineId);
  }

  /** 可设为 active 的条目（排除 whisper-cli 运行时与 VAD） */
  function isSelectableModel(m: ModelInfo): boolean {
    return m.id !== "whisper-cli" && m.engineId !== "vad-silero";
  }

  const vadModels = $derived(models.filter((m) => m.engineId === "vad-silero"));
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">引擎与模型</h1>
  <p class="mt-0.5 text-[11px] text-white/45">识别引擎随时可换，模型本地运行</p>

  <!-- 模型存储位置 -->
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
          class="shrink-0 rounded-lg bg-kotone-cyan px-3 py-1.5 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
          disabled={migrating}
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
      <p class="mt-2 text-[11px] text-white/40">
        已下载的模型会移动到新目录（迁移失败的条目需重新下载）；whisper-cli 运行时固定在 ~/.kotone/bin，不受影响。
      </p>
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
    {/if}
  </section>

  <!-- restartNeeded 提示（与标题栏联动） -->
  {#if $runtimeStore?.restartNeeded}
    <div class="mt-4 flex items-center gap-2 rounded-lg bg-yellow-400/10 px-3 py-2 ring-1 ring-yellow-400/40">
      <span class="h-2 w-2 shrink-0 rounded-full bg-yellow-400"></span>
      <p class="text-[11px] text-yellow-200/90">
        引擎 / 活动模型已变更，运行中的实例仍用旧配置——点标题栏「重启生效」后按新配置运行。
      </p>
    </div>
  {/if}

  {#if engines === null}
    <div class="mt-10 flex flex-col items-center gap-3">
      <img src={stickerCurious} alt="加载中" class="h-24 w-24 object-contain" />
      <p class="text-sm text-white/50">Kotone 正在清点引擎…</p>
    </div>
  {:else if engines.length === 0}
    <div class="mt-10 flex flex-col items-center gap-3">
      <img src={stickerCurious} alt="空空如也" class="h-24 w-24 object-contain" />
      <p class="text-sm text-white/50">还没有可用引擎</p>
    </div>
  {:else}
    <div class="mt-4 flex flex-col gap-3">
      {#each engines as en}
        {@const active = $settingsStore?.sttEngine === en.id}
        {@const enModels = modelsOf(en.id)}
        <div class="kotone-card p-4 {active ? 'border-kotone-cyan/50 shadow-glow-cyan' : ''}">
          <div class="flex items-center gap-3">
            <div class="min-w-0 flex-1">
              <p class="flex items-center gap-2 text-sm font-medium">
                <span class="truncate">{en.displayName}</span>
                {#if active}
                  <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">
                    使用中
                  </span>
                {/if}
              </p>
              <p class="mt-1.5 flex flex-wrap gap-1">
                {#each engineTags(en) as tag}
                  <span class="rounded px-1.5 py-0.5 text-[10px] {tag.cls}">{tag.text}</span>
                {/each}
                <span class="rounded bg-white/5 px-1.5 py-0.5 text-[10px] text-white/40">
                  {en.capabilities.languages.join(" / ")}
                </span>
              </p>
            </div>
            {#if !active}
              <button
                class="shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold transition active:scale-95 {en.isReady
                  ? 'bg-kotone-cyan text-kotone-deep hover:brightness-110'
                  : 'bg-white/10 text-white/60 hover:bg-white/20'}"
                onclick={() => void onEngineSwitch(en.id)}
              >
                {en.isReady ? "切换" : "仍切换"}
              </button>
            {/if}
          </div>

          <!-- 该引擎的模型 / 运行时 -->
          {#each enModels as m}
            {@const isActive = isSelectableModel(m) && activeModelOf(en.id) === m.id}
            <div class="mt-3 rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/8">
              <div class="flex items-center justify-between gap-2">
                <div class="min-w-0 flex-1">
                  <p class="flex items-center gap-1.5 text-[12px] text-white/80">
                    <span class="truncate">{m.displayName}</span>
                    {#if isActive}
                      <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] font-semibold text-kotone-cyan">
                        active
                      </span>
                    {/if}
                  </p>
                  <p class="mt-0.5 text-[10px] text-white/35">
                    {m.id} · {formatSize(m.sizeBytes)}{en.capabilities.streaming && isSelectableModel(m) ? " · 流式" : ""}
                  </p>
                </div>
                <div class="flex shrink-0 items-center gap-1.5">
                  {#if m.downloaded}
                    {#if isSelectableModel(m) && !isActive}
                      <button
                        class="rounded-lg bg-kotone-cyan/15 px-2.5 py-1 text-[11px] font-semibold text-kotone-cyan ring-1 ring-kotone-cyan/40 transition hover:bg-kotone-cyan/25 active:scale-95"
                        onclick={() => void onSetActive(en.id, m.id)}
                      >
                        设为 active
                      </button>
                    {/if}
                    {#if confirmingDelete === m.id}
                      <button
                        class="rounded-lg bg-kotone-pink px-2.5 py-1 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95 disabled:opacity-50"
                        disabled={deleting !== null}
                        onclick={() => void onDelete(m.id)}
                      >
                        {deleting === m.id ? "删除中…" : "确认删除"}
                      </button>
                      <button
                        class="rounded-lg bg-white/10 px-2 py-1 text-[11px] text-white/60 transition hover:bg-white/20"
                        onclick={() => (confirmingDelete = null)}
                      >
                        取消
                      </button>
                    {:else}
                      <button
                        class="rounded-lg bg-white/8 px-2.5 py-1 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-kotone-pink/20 hover:text-kotone-pink active:scale-95"
                        onclick={() => (confirmingDelete = m.id)}
                      >
                        删除
                      </button>
                    {/if}
                  {:else}
                    <button
                      class="rounded-lg bg-kotone-violet/25 px-2.5 py-1 text-[11px] font-semibold text-kotone-violet ring-1 ring-kotone-violet/40 transition hover:bg-kotone-violet/35 active:scale-95 disabled:opacity-50"
                      disabled={downloadingAny}
                      onclick={() => void onDownload(m.id)}
                    >
                      {m.id in dlProgress ? "下载中…" : "下载"}
                    </button>
                  {/if}
                </div>
              </div>
              {#if m.id in dlProgress}
                <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-white/10">
                  {#if dlProgress[m.id] !== null}
                    <div
                      class="h-full rounded-full bg-kotone-violet transition-[width]"
                      style:width="{dlProgress[m.id]}%"
                    ></div>
                  {:else}
                    <div class="h-full w-1/3 animate-pulse rounded-full bg-kotone-violet/70"></div>
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/each}

      <!-- VAD 组件组（不可设为 active，one-shot 静音判停依赖） -->
      {#if vadModels.length > 0}
        <div class="kotone-card p-4">
          <p class="text-sm font-medium">VAD 组件</p>
          <p class="mt-1 text-[10px] text-white/40">one-shot「说一句就走」的静音判停依赖；不属于任何识别引擎</p>
          {#each vadModels as m}
            <div class="mt-3 rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/8">
              <div class="flex items-center justify-between gap-2">
                <div class="min-w-0 flex-1">
                  <p class="text-[12px] text-white/80">{m.displayName}</p>
                  <p class="mt-0.5 text-[10px] text-white/35">{m.id} · {formatSize(m.sizeBytes)}</p>
                </div>
                <div class="flex shrink-0 items-center gap-1.5">
                  {#if m.downloaded}
                    {#if confirmingDelete === m.id}
                      <button
                        class="rounded-lg bg-kotone-pink px-2.5 py-1 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95 disabled:opacity-50"
                        disabled={deleting !== null}
                        onclick={() => void onDelete(m.id)}
                      >
                        {deleting === m.id ? "删除中…" : "确认删除"}
                      </button>
                      <button
                        class="rounded-lg bg-white/10 px-2 py-1 text-[11px] text-white/60 transition hover:bg-white/20"
                        onclick={() => (confirmingDelete = null)}
                      >
                        取消
                      </button>
                    {:else}
                      <button
                        class="rounded-lg bg-white/8 px-2.5 py-1 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-kotone-pink/20 hover:text-kotone-pink active:scale-95"
                        onclick={() => (confirmingDelete = m.id)}
                      >
                        删除
                      </button>
                    {/if}
                  {:else}
                    <button
                      class="rounded-lg bg-kotone-violet/25 px-2.5 py-1 text-[11px] font-semibold text-kotone-violet ring-1 ring-kotone-violet/40 transition hover:bg-kotone-violet/35 active:scale-95 disabled:opacity-50"
                      disabled={downloadingAny}
                      onclick={() => void onDownload(m.id)}
                    >
                      {m.id in dlProgress ? "下载中…" : "下载"}
                    </button>
                  {/if}
                </div>
              </div>
              {#if m.id in dlProgress}
                <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-white/10">
                  {#if dlProgress[m.id] !== null}
                    <div
                      class="h-full rounded-full bg-kotone-violet transition-[width]"
                      style:width="{dlProgress[m.id]}%"
                    ></div>
                  {:else}
                    <div class="h-full w-1/3 animate-pulse rounded-full bg-kotone-violet/70"></div>
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
    <p class="mt-3 text-[11px] text-white/40">
      未就绪的引擎（模型未下载）「启动」时会报出具体缺失项；切换引擎 / 活动模型后如已「启动」，需点标题栏「重启生效」。
    </p>
  {/if}
</div>
