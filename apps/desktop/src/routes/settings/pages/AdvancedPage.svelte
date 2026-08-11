<script lang="ts">
  /*
   * 高级页：
   * - 模型存储位置（更改[迁移]/打开目录）→ 完整模型清单（下载进度/删除二次确认/设为 active）；
   *   产品只内置一套 sherpa 识别方案，不再向用户暴露「引擎」概念；
   * - restartNeeded 黄条常显（与标题栏「重启生效」联动）；
   * - 语言（i18n 伏笔，当前仅中文）、频道切换热键（ADR-008）、运行时、Windows 兼容性等进阶设置；
   * - mock 联调引擎与 VAD 组件不在此页出现（VAD 已随应用本体分发，无需下载）。
   */
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    listSttEngines,
    exportDiagnostics,
    listModels,
    downloadModel,
    setActiveModel,
    deleteModel,
    getModelsDir,
    setModelsDir,
    openModelsDir,
    openHistoryDir,
    cancelDownload,
    updateSettings,
    getElevationStatus,
    getHotkeyStatus,
    restartAsAdmin,
    isTauri,
    type DownloadProgress,
    type ElevationStatus,
    type EngineInfo,
    type HotkeyStatus,
    type ModelInfo,
    type ModelsDirInfo,
  } from "../../../lib/ipc";
  import {
    settingsStore,
    patchSettings,
    toast,
    toastWarn,
    errText,
  } from "../../../lib/stores/ui";
  import { runtimeStore } from "../../../lib/stores/runtime";
  import { spotlight } from "../../../lib/actions/spotlight";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import { captureHotkey } from "../../../lib/hotkeyCapture";
  import { combosConflict } from "../../../lib/hotkeyCombo";
  import stickerCurious from "../../../assets/brand/stickers/curious.webp";

  /** 设置助手入口：由 Settings 打开首启向导（自「通用」页迁入） */
  let { onOpenOnboarding }: { onOpenOnboarding: () => void } = $props();

  let engines = $state<EngineInfo[] | null>(null);
  let models = $state<ModelInfo[]>([]);
  let dirInfo = $state<ModelsDirInfo | null>(null);
  /** 下载中模型 id → 进度百分比（null = 连接中） */
  let dlProgress = $state<Record<string, number | null>>({});
  /** 下载失败模型 id → 可持续显示的错误，直到重试或成功。 */
  let dlErrors = $state<Record<string, string>>({});
  /** 二次确认删除中的模型 id */
  let confirmingDelete = $state<string | null>(null);
  let deleting = $state<string | null>(null);
  /** 目录更改模式：输入草稿 */
  let editingDir = $state(false);
  let dirDraft = $state("");
  let migrating = $state(false);
  let elevation = $state<ElevationStatus | null>(null);
  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  let restartingAsAdmin = $state(false);
  /** 诊断包导出中（从「关于」页迁入：排障入口归高级页） */
  let exportingDiagnostics = $state(false);
  let downloadProxyDraft = $state("");
  /** 频道切换热键（ADR-008）：编辑草稿 + 录入捕获态 */
  let cycleDraft = $state("Shift+CapsLock");
  let cycleCapturing = $state(false);
  let cycleCleanup: (() => void) | null = null;
  /** 重发最近一条热键：编辑草稿 + 录入捕获态 */
  let resendDraft = $state("");
  let resendCapturing = $state(false);
  let resendCleanup: (() => void) | null = null;

  const ADMIN_RESTART_FLAG = "kotone:admin-restart-pending";

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);

  onMount(() => {
    let un: (() => void) | undefined;
    void (async () => {
      await reload();
      [elevation, hotkeyStatus] = await Promise.all([
        getElevationStatus().catch(() => null),
        getHotkeyStatus().catch(() => null),
      ]);
      downloadProxyDraft = $settingsStore?.download.ghProxy ?? "";
      cycleDraft = $settingsStore?.channelCycleHotkey ?? "Shift+CapsLock";
      resendDraft = $settingsStore?.resendLastHotkey ?? "";
      if (localStorage.getItem(ADMIN_RESTART_FLAG)) {
        localStorage.removeItem(ADMIN_RESTART_FLAG);
        if (elevation?.elevated) toast(true, "已通过管理员权限运行");
      }
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

  /** 三项独立加载：单项失败只 toast + console.error，不拖垮整页（曾因 Promise.all
   *  任一失败把 engines 清空，整页显示「还没有可用模型」掩盖真实错误） */
  async function reload() {
    const [en, mo, di] = await Promise.all([
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
    ]);
    if (en === null && mo === null && di === null) {
      // 全灭（通常是壳 IPC 整体故障）：给空态而非半截页面
      engines = [];
      return;
    }
    if (en !== null) engines = en;
    if (mo !== null) models = mo;
    if (di !== null) dirInfo = di;
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
        toastWarn(`已切换，但 ${report.failed.length} 项迁移失败（需重新下载）：${report.failed.join(", ")}`);
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

  /** 文件夹选择器（用户反馈：模型路径应支持可视化选择而非手输） */
  async function onBrowseDir() {
    const sel = await openDialog({ directory: true, title: "选择模型存储位置" }).catch(() => null);
    if (!sel) return; // 用户取消
    dirDraft = sel;
  }

  // ---------- 历史记录位置（P2-⑨） ----------

  /** 当前历史目录（自定义或默认路径拼接；仅展示用） */
  const historyDirLabel = $derived(
    $settingsStore?.history.dir.trim() || "~/.kotone/history",
  );
  const historyDirCustom = $derived(Boolean($settingsStore?.history.dir.trim()));

  async function onChangeHistoryDir() {
    const sel = await openDialog({ directory: true, title: "选择历史记录位置" }).catch(() => null);
    if (!sel) return; // 用户取消
    await patchSettings({ history: { dir: sel } }, "历史目录已切换");
  }

  async function onOpenHistoryDir() {
    try {
      await openHistoryDir();
    } catch (e) {
      toast(false, `打开目录失败：${errText(e)}`);
    }
  }

  // ---------- 模型操作 ----------

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
      dlErrors[id] = errText(e);
      toast(false, "下载失败，可在模型卡片中重试或调整下载源");
    } finally {
      delete dlProgress[id];
    }
  }

  /** 取消进行中的下载（.part 保留可续传；P2-⑦） */
  async function onCancelDownload(id: string) {
    try {
      await cancelDownload();
    } catch (e) {
      toast(false, `取消失败：${errText(e)}`);
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
      if ($runtimeStore?.phase === "running") {
        toastWarn(`活动模型已切换：${modelId}，重启后生效`);
      } else {
        toast(true, `活动模型已切换：${modelId}`);
      }
    } catch (e) {
      toast(false, `切换活动模型失败：${errText(e)}`);
    }
  }

  // ---------- 展示辅助 ----------

  function formatSize(bytes: number): string {
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  /** 引擎当前活动模型 id（engineOptions 未配置按引擎清单默认） */
  function activeModelOf(engineId: string): string {
    const configured = ($settingsStore?.engineOptions?.[engineId] as Record<string, unknown> | undefined)
      ?.model as string | undefined;
    if (configured) return configured;
    if (engineId === "sherpa-onnx-x-asr-zh-en") return "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05";
    if (engineId === "sherpa-onnx-sensevoice") return "sense-voice-zh-en-ja-ko-yue-2024-07-17";
    if (engineId === "sherpa-onnx-funasr-nano") return "funasr-nano-int8-2025-12-30";
    return "default";
  }

  function modelsOf(engineId: string): ModelInfo[] {
    return models.filter((m) => m.engineId === engineId);
  }

  /** 固定产品顺序：X-ASR → SenseVoice → FunASR；mock 联调引擎不展示。 */
  const engineOrder: Record<string, number> = {
    "sherpa-onnx-x-asr-zh-en": 0,
    "sherpa-onnx-sensevoice": 1,
    "sherpa-onnx-funasr-nano": 2,
  };
  const orderedEngines = $derived(
    (engines ?? [])
      .filter((e) => e.id !== "mock-stream")
      .slice()
      .sort((a, b) => (engineOrder[a.id] ?? 99) - (engineOrder[b.id] ?? 99)),
  );

  /** 主力引擎：只恒显 X-ASR；SenseVoice / FunASR 收进「备用」折叠区 */
  const PRIMARY_ENGINE_ID = "sherpa-onnx-x-asr-zh-en";
  const primaryEngines = $derived(orderedEngines.filter((e) => e.id === PRIMARY_ENGINE_ID));
  const otherEngines = $derived(orderedEngines.filter((e) => e.id !== PRIMARY_ENGINE_ID));
  let showOtherEngines = $state(false);

  /** 高级调参区（VAD 帧级判定 + 热词权重）：默认折叠，避免干扰 */
  let showAdvancedTuning = $state(false);

  async function onHotkeyBackendChange(e: Event) {
    const hotkeyBackend = (e.target as HTMLSelectElement).value;
    await patchSettings({ hotkeyBackend }, "热键兼容模式已更新");
    hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
  }

  async function saveDownloadProxy() {
    await patchSettings(
      { download: { ghProxy: downloadProxyDraft.trim() } },
      "下载代理地址已保存",
    );
  }

  /** 保存频道切换热键：先与录制热键做前端冲突预检（后端注册仍有双保险） */
  async function saveCycleHotkey() {
    const key = cycleDraft.trim();
    if (!key) {
      toast(false, "频道切换热键不能为空");
      return;
    }
    const recordKey = $settingsStore?.hotkey.key ?? "";
    if (recordKey && combosConflict(key, recordKey)) {
      toast(false, `频道切换热键与录制热键（${recordKey}）冲突，请换一个`);
      return;
    }
    try {
      settingsStore.set(await updateSettings({ channelCycleHotkey: key }));
      cycleDraft = $settingsStore?.channelCycleHotkey ?? key;
      toast(true, `频道切换热键已保存并生效：${cycleDraft}`);
    } catch (e) {
      toast(false, `保存频道切换热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  /** 「点击录入」：与通用页录制热键共用 LL 钩子捕获 helper */
  async function startCycleCapture() {
    if (cycleCapturing) return;
    cycleCapturing = true;
    cycleCleanup = await captureHotkey((r) => {
      cycleCapturing = false;
      cycleCleanup = null;
      if (r.kind === "combo") {
        cycleDraft = r.combo;
        void saveCycleHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  /** 保存重发最近一条热键：先与录制/频道切换热键做前端冲突预检（后端注册仍有双保险） */
  async function saveResendHotkey() {
    const key = resendDraft.trim();
    if (!key) {
      toast(false, "重发热键不能为空（留空 = 关闭功能，可点清除）");
      return;
    }
    const recordKey = $settingsStore?.hotkey.key ?? "";
    const cycleKey = $settingsStore?.channelCycleHotkey ?? "";
    if (recordKey && combosConflict(key, recordKey)) {
      toast(false, `重发热键与录制热键（${recordKey}）冲突，请换一个`);
      return;
    }
    if (cycleKey && combosConflict(key, cycleKey)) {
      toast(false, `重发热键与频道切换热键（${cycleKey}）冲突，请换一个`);
      return;
    }
    try {
      settingsStore.set(await updateSettings({ resendLastHotkey: key }));
      resendDraft = $settingsStore?.resendLastHotkey ?? key;
      toast(true, `重发热键已保存并生效：${resendDraft}`);
    } catch (e) {
      toast(false, `保存重发热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  /** 「点击录入」重发热键（与频道切换键同构） */
  async function startResendCapture() {
    if (resendCapturing) return;
    resendCapturing = true;
    resendCleanup = await captureHotkey((r) => {
      resendCapturing = false;
      resendCleanup = null;
      if (r.kind === "combo") {
        resendDraft = r.combo;
        void saveResendHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  /** 清除重发热键（留空 = 关闭功能） */
  async function clearResendHotkey() {
    resendDraft = "";
    try {
      settingsStore.set(await updateSettings({ resendLastHotkey: "" }));
      toast(true, "已关闭重发热键");
    } catch (e) {
      toast(false, `清除重发热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  onDestroy(() => {
    cycleCleanup?.();
    resendCleanup?.();
  });

  /** 导出诊断包（不含录音、识别文本和热词），可安全发给测试群管理员 */
  async function onExportDiagnostics() {
    if (exportingDiagnostics) return;
    exportingDiagnostics = true;
    try {
      const defaultName = `kotone-diagnostics-${new Date().toISOString().slice(0, 10)}.zip`;
      const path = isTauri
        ? await saveDialog({
            title: "导出 Kotone 诊断包",
            defaultPath: defaultName,
            filters: [{ name: "ZIP 诊断包", extensions: ["zip"] }],
          })
        : defaultName;
      if (!path) return;
      const result = await exportDiagnostics(path);
      toast(true, `诊断包已导出：${result.reportId}`);
    } catch (error) {
      toast(false, `导出诊断包失败：${errText(error)}`);
    } finally {
      exportingDiagnostics = false;
    }
  }

  async function onRestartAsAdmin() {
    restartingAsAdmin = true;
    try {
      localStorage.setItem(ADMIN_RESTART_FLAG, "1");
      await restartAsAdmin();
    } catch (error) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      restartingAsAdmin = false;
      toast(false, errText(error));
    }
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">高级</h1>
  <p class="mt-0.5 text-[11px] text-white/45">模型、频道与 Windows 兼容性设置</p>

  <!-- restartNeeded 提示（与标题栏联动） -->
  {#if $runtimeStore?.restartNeeded}
    <div class="mt-4 flex items-center gap-2 rounded-lg bg-yellow-400/10 px-3 py-2 ring-1 ring-yellow-400/40">
      <span class="h-2 w-2 shrink-0 rounded-full bg-yellow-400"></span>
      <p class="text-[11px] text-yellow-200/90">
        活动模型已变更，运行中的实例仍用旧配置——点标题栏「重启生效」后按新配置运行。
      </p>
    </div>
  {/if}

  {#if $settingsStore}
    <!-- 语言（i18n 伏笔：当前仅中文，后续接入界面文案多语言） -->
    <section class="kotone-panel mt-4 flex items-center justify-between gap-4 p-4">
      <div>
        <h2 class="text-sm font-semibold text-kotone-cyan/90">语言</h2>
        <p class="mt-1 text-[11px] leading-relaxed text-white/50">界面语言。更多语言即将到来。</p>
      </div>
      <select
        class="shrink-0 rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
        value={$settingsStore.language}
        onchange={(e) =>
          void patchSettings({ language: (e.target as HTMLSelectElement).value }, "语言已切换")}
      >
        <option value="zh">中文</option>
      </select>
    </section>

    <!-- 高级调参（VAD 帧级判定 + 热词权重）：默认折叠，避免干扰 -->
    <button
      class="kotone-panel mt-4 flex w-full cursor-pointer items-center justify-between gap-3 p-4 text-left transition hover:bg-white/5"
      onclick={() => (showAdvancedTuning = !showAdvancedTuning)}
      aria-expanded={showAdvancedTuning}
    >
      <div>
        <h2 class="text-sm font-semibold text-kotone-cyan/90">高级调参</h2>
        <p class="mt-0.5 text-[11px] text-white/45">
          VAD 帧级判定参数与热词权重（默认值即原行为）
        </p>
      </div>
      <span
        class="shrink-0 text-white/40 transition-transform {showAdvancedTuning ? 'rotate-180' : ''}"
      >
        ▾
      </span>
    </button>

    {#if showAdvancedTuning}
      <div class="mt-2 flex flex-col gap-3">
        <div class="flex items-start gap-2 rounded-lg bg-yellow-400/10 px-3 py-2 ring-1 ring-yellow-400/30">
          <span class="mt-0.5 text-xs text-yellow-300">⚠</span>
          <p class="text-[11px] leading-relaxed text-yellow-200/90">
            除非你知道你在做什么，否则不建议修改这些配置。
          </p>
        </div>

        <!-- VAD 高级设置：silero 帧级判定参数（区别于通用页「静音判停时长」） -->
        <section class="kotone-panel flex flex-col gap-4 p-4">
          <div>
            <h2 class="text-sm font-semibold text-kotone-cyan/90">VAD 高级设置</h2>
            <p class="mt-1 text-[11px] leading-relaxed text-white/45">
              控制 silero 把背景噪声判成说话的程度，仅 one-shot / solo 生效（与通用页的「静音判停时长」是两回事）。下一条会话立即生效。
            </p>
          </div>
          <label class="block">
            <div class="flex items-center justify-between">
              <span class="text-xs text-white/70">语音判定阈值</span>
              <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
                {$settingsStore.vad.threshold.toFixed(2)}
              </span>
            </div>
            <input
              type="range"
              min="0.1"
              max="0.9"
              step="0.05"
              value={$settingsStore.vad.threshold}
              onchange={(e) =>
                void patchSettings(
                  { vad: { threshold: Number((e.target as HTMLInputElement).value) } },
                  "VAD 阈值已保存",
                )}
              class="mt-2 w-full accent-kotone-cyan"
            />
          </label>
          <label class="block">
            <div class="flex items-center justify-between">
              <span class="text-xs text-white/70">最短语音</span>
              <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
                {$settingsStore.vad.minSpeechMs} ms
              </span>
            </div>
            <input
              type="range"
              min="20"
              max="500"
              step="10"
              value={$settingsStore.vad.minSpeechMs}
              onchange={(e) =>
                void patchSettings(
                  { vad: { minSpeechMs: Number((e.target as HTMLInputElement).value) } },
                  "最短语音已保存",
                )}
              class="mt-2 w-full accent-kotone-cyan"
            />
          </label>
          <label class="block">
            <div class="flex items-center justify-between">
              <span class="text-xs text-white/70">最短静音</span>
              <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
                {$settingsStore.vad.minSilenceMs} ms
              </span>
            </div>
            <input
              type="range"
              min="20"
              max="500"
              step="10"
              value={$settingsStore.vad.minSilenceMs}
              onchange={(e) =>
                void patchSettings(
                  { vad: { minSilenceMs: Number((e.target as HTMLInputElement).value) } },
                  "最短静音已保存",
                )}
              class="mt-2 w-full accent-kotone-cyan"
            />
          </label>
          <p class="text-[10px] leading-relaxed text-white/35">
            调高阈值、拉长最短语音可减少背景噪声被误判成语音；默认值即原行为。
          </p>
        </section>

        <!-- 热词权重：X-ASR 热词加分（句首噪声幻觉「DPS」可调低） -->
        <section class="kotone-panel flex flex-col gap-4 p-4">
          <div>
            <h2 class="text-sm font-semibold text-kotone-cyan/90">热词权重</h2>
            <p class="mt-1 text-[11px] leading-relaxed text-white/45">
              热词命中加分：越高热词越容易命中，但背景噪声被识别成热词的概率也越高（如句首「DPS」）。
            </p>
          </div>
          <label class="block">
            <div class="flex items-center justify-between">
              <span class="text-xs text-white/70">热词加分</span>
              <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
                {$settingsStore.hotwordsScore.toFixed(1)}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="10"
              step="0.5"
              value={$settingsStore.hotwordsScore}
              onchange={(e) =>
                void patchSettings(
                  { hotwordsScore: Number((e.target as HTMLInputElement).value) },
                  "热词权重已保存",
                )}
              class="mt-2 w-full accent-kotone-cyan"
            />
          </label>
          <p class="text-[10px] leading-relaxed text-white/35">
            0 = 无偏置（完全不鼓励热词）；改动需「停止 → 启动」后生效。
          </p>
        </section>
      </div>
    {/if}
  {/if}

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
          class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/75 ring-1 ring-white/15 transition hover:bg-white/20"
          onclick={() => void onBrowseDir()}
        >
          浏览…
        </button>
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
        已下载的模型会移动到新目录（迁移失败的条目需重新下载）。
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

  <!-- 历史记录位置（P2-⑨：历史音频也可能占大量空间，支持自定义目录） -->
  <section class="kotone-panel mt-4 p-4">
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
  {:else if engines.length === 0}
    <div class="mt-10 flex flex-col items-center gap-3">
      <img src={stickerCurious} alt="空空如也" class="h-24 w-24 object-contain" />
      <p class="text-sm text-white/50">还没有可用模型</p>
    </div>
  {:else}
    {#snippet engineCard(en: EngineInfo, showName: boolean)}
        {@const enModels = modelsOf(en.id)}
        <div use:spotlight class="kotone-card kotone-spotlight p-4">
          {#if showName}
            <p class="mb-2 text-[11px] font-semibold text-white/55">{en.displayName}</p>
          {/if}
          <!-- 模型：恒显完整清单（含未下载）——已下载行 radio 可选，未下载行置灰 + 行尾下载 -->
          {#if enModels.length > 0}
            <p class="mt-3 text-[10px] font-semibold tracking-wide text-white/40">
              模型（点击已下载行即切换）
            </p>
            <div class="mt-1.5 flex flex-col gap-1.5" role="radiogroup" aria-label="{en.displayName} 模型选择">
              {#each enModels as m}
                {@const isActive = activeModelOf(en.id) === m.id}
                {#if m.downloaded}
                  <div
                    role="radio"
                    aria-checked={isActive}
                    tabindex="0"
                    class="flex cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 ring-1 transition
                      {isActive
                      ? 'bg-kotone-cyan/10 ring-kotone-cyan/60 shadow-glow-cyan'
                      : 'bg-white/4 ring-white/10 hover:bg-white/8 hover:shadow-[0_0_12px_rgba(0,229,255,0.14)] hover:ring-white/25'}"
                    onclick={() => !isActive && void onSetActive(en.id, m.id)}
                    onkeydown={(e) => {
                      if (e.key === "Enter" && !isActive) void onSetActive(en.id, m.id);
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
                        {m.id} · {formatSize(m.sizeBytes)}{en.capabilities.streaming ? " · 流式" : ""}
                      </p>
                    </div>
                    {#if isActive}
                      <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] font-semibold text-kotone-cyan">
                        active
                      </span>
                    {/if}
                    {#if confirmingDelete === m.id}
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
                    {:else}
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
                  <!-- 未下载项本身不可选择，但下载/重试操作必须保持可访问。 -->
                  <div class="rounded-lg bg-white/3 px-3 py-2 ring-1 ring-white/8">
                    <div class="flex items-center gap-2.5">
                      <span class="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full ring-2 ring-white/15"></span>
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-[12px] text-white/45">{m.displayName}</p>
                        <p class="mt-0.5 text-[10px] text-white/30">
                          {m.id} · {formatSize(m.sizeBytes)}{en.capabilities.streaming ? " · 流式" : ""}
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
                          class="shrink-0 rounded-lg bg-white/10 px-2.5 py-1 text-[11px] text-white/70 ring-1 ring-white/15 transition hover:bg-white/20"
                          onclick={() => void onCancelDownload(m.id)}
                        >
                          取消
                        </button>
                      </div>
                    {/if}
                    {#if dlErrors[m.id]}
                      <div class="mt-2 flex items-start gap-2 rounded-lg bg-kotone-pink/10 px-2.5 py-2 ring-1 ring-kotone-pink/35">
                        <p class="min-w-0 flex-1 text-[10px] leading-relaxed break-all text-kotone-pink">
                          下载失败：{dlErrors[m.id]}
                        </p>
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
          {/if}

        </div>
    {/snippet}

    <!-- 主力模型：X-ASR 恒显 -->
    <div class="mt-4 flex flex-col gap-3">
      {#each primaryEngines as en}
        {@render engineCard(en, false)}
      {/each}
    </div>

    <!-- 备用识别模型：默认折叠，避免干扰（单引擎产品策略） -->
    {#if otherEngines.length > 0}
      <button
        class="mt-3 self-start rounded-lg bg-white/6 px-3 py-1.5 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-white/12 hover:text-white/85"
        onclick={() => (showOtherEngines = !showOtherEngines)}
      >
        {showOtherEngines ? "收起备用识别模型" : `其他识别模型（${otherEngines.length} 套备用，通常不需要）`}
      </button>
      {#if showOtherEngines}
        <div class="mt-3 flex flex-col gap-3">
          {#each otherEngines as en}
            {@render engineCard(en, true)}
          {/each}
        </div>
      {/if}
    {/if}
    <p class="mt-3 text-[11px] text-white/40">
      模型未下载时「启动」会报出具体缺失项；切换活动模型后如已「启动」，需点标题栏「重启生效」。
    </p>
  {/if}

  {#if $settingsStore}
    <!-- 运行时（自「通用」页迁入：自动启动属于进阶偏好） -->
    <section class="kotone-panel mt-4 flex flex-col gap-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">运行时</h2>
      <Toggle
        checked={$settingsStore.ui.autoStart}
        label="启动 Kotone 后自动开始运行"
        desc="自动加载识别模型、注册热键并显示悬浮窗；关闭时需手动点「启动」"
        onchange={(v) =>
          void patchSettings({ ui: { autoStart: v } }, v ? "已开启自动启动" : "已关闭自动启动，需手动点「启动」")}
      />
    </section>

    <!-- 频道切换热键（ADR-008）：循环切换当前游戏适配声明的聊天频道 -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">频道切换热键</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">
        支持多频道的游戏适配（如英雄联盟的「队伍 / 所有人」）按声明顺序循环切换，悬浮窗会显示当前频道。
      </p>
      <div class="mt-3 flex items-center gap-2">
        <input
          bind:value={cycleDraft}
          disabled={cycleCapturing}
          class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
          placeholder="如 Shift+CapsLock"
          spellcheck="false"
          onkeydown={(e) => {
            if (e.key === "Enter" && !cycleCapturing) void saveCycleHotkey();
          }}
        />
        <button
          class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {cycleCapturing
            ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
            : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
          disabled={cycleCapturing}
          onclick={() => void startCycleCapture()}
        >
          {cycleCapturing ? "请按下热键组合…（Esc 取消）" : "点击录入"}
        </button>
      </div>
      {#if hotkeyStatus?.cycleError}
        <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{hotkeyStatus.cycleError}</p>
      {:else if hotkeyStatus?.cycleKey}
        <p class="mt-2 text-[11px] text-white/40">当前生效：{hotkeyStatus.cycleKey}</p>
      {/if}
      <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">
        不能与「通用」页的录制热键相同；当前游戏适配只有一个频道时该键不生效。
      </p>
    </section>

    <!-- 重发最近一条热键：空闲时一键把历史最新一条发送文本重新注入当前前台窗口 -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">重发最近一条热键</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">
        空闲时按下，把历史记录里最新一条发送成功的文本重新发送到当前前台窗口
        （用当前游戏适配与频道；正在说话/发送时按下不生效）。
      </p>
      <div class="mt-3 flex items-center gap-2">
        <input
          bind:value={resendDraft}
          disabled={resendCapturing}
          class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
          placeholder="如 Alt+F6"
          spellcheck="false"
          onkeydown={(e) => {
            if (e.key === "Enter" && !resendCapturing) void saveResendHotkey();
          }}
        />
        <button
          class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {resendCapturing
            ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
            : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
          disabled={resendCapturing}
          onclick={() => void startResendCapture()}
        >
          {resendCapturing ? "请按下热键组合…（Esc 取消）" : "点击录入"}
        </button>
        {#if resendDraft}
          <button
            class="rounded-lg px-2.5 py-1.5 text-xs text-white/60 ring-1 ring-white/15 transition hover:bg-white/10 active:scale-95"
            onclick={() => void clearResendHotkey()}
          >
            清除
          </button>
        {/if}
      </div>
      {#if hotkeyStatus?.resendError}
        <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{hotkeyStatus.resendError}</p>
      {:else if hotkeyStatus?.resendKey}
        <p class="mt-2 text-[11px] text-white/40">当前生效：{hotkeyStatus.resendKey}</p>
      {:else if !resendDraft}
        <p class="mt-2 text-[11px] text-white/40">未设置（默认关闭，不会误触发）</p>
      {/if}
      <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">
        不能与「通用」页的录制热键或频道切换热键相同；历史中没有发送成功的文本时按下无效果。
      </p>
    </section>

    <!-- 设置助手（自「通用」页迁入） -->
    <section class="kotone-panel mt-4 flex items-center justify-between gap-4 p-4">
      <div>
        <h2 class="text-sm font-semibold text-kotone-cyan/90">设置助手</h2>
        <p class="mt-1 text-[11px] leading-relaxed text-white/50">
          重新选择游戏配置、检查模型与麦克风、设置热键，并完成一次真实发送测试。
        </p>
      </div>
      <button
        class="shrink-0 rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-kotone-cyan"
        onclick={onOpenOnboarding}
      >
        重新运行向导
      </button>
    </section>

    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">悬浮窗外观</h2>
      <p class="mt-1 text-[11px] text-white/45">只改变展示密度，不影响识别和发送。</p>
      <div class="mt-3 grid grid-cols-2 gap-2">
        {#each [
          { id: "capsule", name: "胶囊", desc: "更轻巧，减少遮挡" },
          { id: "card", name: "卡片", desc: "信息完整，适合首次使用" },
        ] as option}
          {@const selected = $settingsStore.overlay.style === option.id}
          <button
            class="rounded-lg px-3 py-2.5 text-left ring-1 transition {selected
              ? 'bg-kotone-violet/15 ring-kotone-violet/60'
              : 'bg-white/5 ring-white/10 hover:bg-white/10'}"
            onclick={() =>
              void patchSettings({ overlay: { style: option.id } }, `悬浮窗外观已切换：${option.name}`)}
          >
            <p class="text-sm font-semibold {selected ? 'text-kotone-violet' : ''}">{option.name}</p>
            <p class="mt-0.5 text-[11px] text-white/45">{option.desc}</p>
          </button>
        {/each}
      </div>
    </section>

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

    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">Windows 兼容性</h2>
      <div class="mt-3 flex items-center gap-2">
        <label class="text-xs text-white/60" for="advanced-hotkey-backend">热键实现</label>
        <select
          id="advanced-hotkey-backend"
          class="rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
          value={$settingsStore.hotkeyBackend}
          onchange={(event) => void onHotkeyBackendChange(event)}
        >
          <option value="auto">自动（推荐）</option>
          <option value="llhook">低层键盘钩子</option>
          <option value="register">系统热键</option>
        </select>
        <span class="text-[11px] text-white/40">
          当前：{hotkeyStatus?.backend === "llhook"
            ? "低层钩子"
            : hotkeyStatus?.backend === "register"
              ? "系统热键"
              : "未注册"}
        </span>
      </div>
      <p class="mt-2 text-[11px] text-white/40">除非特定游戏收不到热键，否则保持“自动”。</p>

      <div class="mt-4 border-t border-white/8 pt-4">
        <div class="flex items-center justify-between">
          <div>
            <p class="text-xs font-semibold">当前进程权限</p>
            <p class="mt-0.5 text-[11px] text-white/40">
              {elevation === null ? "检测中…" : elevation.elevated ? "管理员" : "普通用户"}
            </p>
          </div>
          {#if elevation && !elevation.elevated}
            <button
              class="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/75 hover:bg-white/20 disabled:opacity-50"
              disabled={restartingAsAdmin}
              onclick={() => void onRestartAsAdmin()}
            >
              {restartingAsAdmin ? "正在重启…" : "以管理员身份重启"}
            </button>
          {/if}
        </div>
        <div class="mt-4">
          <Toggle
            checked={$settingsStore.runAsAdminOnStart}
            label="启动时自动请求管理员权限"
            desc="开启后每次启动都会自动发起 Windows UAC 确认；普通桌面应用不能静默获得管理员权限"
            onchange={(value) =>
              void patchSettings(
                {
                  runAsAdminOnStart: value,
                  ...(value ? { adminPromptDismissed: true, autoAdminPromptDismissed: true } : {}),
                },
                value ? "已开启启动时自动权限请求" : "已关闭启动时自动权限请求",
              )}
          />
        </div>
      </div>
    </section>
  {/if}

  <!-- 诊断包导出（自「关于」页迁入：排障入口归高级页） -->
  <section class="kotone-panel mt-4 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">诊断包</h2>
    <p class="mt-1 text-[11px] text-white/45">
      不包含录音、识别文本和热词，可安全分享给测试群管理员。
    </p>
    <button
      class="mt-3 inline-flex items-center gap-2 rounded-xl bg-kotone-cyan/12 px-4 py-2.5 text-xs font-semibold text-kotone-cyan
        ring-1 ring-kotone-cyan/25 transition hover:bg-kotone-cyan/18 hover:shadow-glow-cyan
        disabled:cursor-wait disabled:opacity-60"
      disabled={exportingDiagnostics}
      onclick={() => void onExportDiagnostics()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4" aria-hidden="true">
        <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 18v2h14v-2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
      {exportingDiagnostics ? "正在生成…" : "导出诊断包"}
    </button>
  </section>
</div>
