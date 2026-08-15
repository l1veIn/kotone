<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    checkInputEnvironment,
    cancelDownload,
    detectHotkeyConflicts,
    downloadModel,
    getModelsDir,
    isTauri,
    listAudioDevices,
    listModels,
    listProfiles,
    modelsDirPathError,
    openModelsDir,
    setAudioDevice,
    setModelsDir,
    updateSettings,
    getModelInstallGuide,
    isDownloadCancelled,
    type AudioDevice,
    type ModelInstallGuide,
    type DownloadProgress,
    type GameProfile,
    type InputEnvironmentCheck,
    type InteractionMode,
    type ModelInfo,
    type ModelsDirInfo,
  } from "../../lib/ipc";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { captureHotkey } from "../../lib/hotkeyCapture";
  import {
    errText,
    settingsStore,
    toast,
    toastInfo,
    toastWarn,
  } from "../../lib/stores/ui";
  import heroOnboarding from "../../assets/brand/hero-onboarding.webp";
  import stickerHello from "../../assets/brand/stickers/hello.webp";
  import stickerThinking from "../../assets/brand/stickers/thinking.webp";
  import stickerCheering from "../../assets/brand/stickers/cheering.webp";
  import ManualDownloadDialog from "../../lib/components/ManualDownloadDialog.svelte";

  let { onDone }: { onDone: () => void } = $props();

  let step = $state(0);
  let loadingResources = $state(true);
  let resourceError = $state("");
  let models = $state<ModelInfo[]>([]);
  let profiles = $state<GameProfile[]>([]);
  let devices = $state<AudioDevice[]>([]);
  let downloadedIds = $state<string[]>([]);
  /** 模型存储位置（用户反馈：下载模型应支持选择路径，而不是默认 C 盘） */
  let dirInfo = $state<ModelsDirInfo | null>(null);
  let changingDir = $state(false);

  const primaryModel = $derived(
    models.find((m) => m.engineId === "sherpa-streaming") ?? null,
  );
  const isDownloaded = (m: ModelInfo | null) =>
    m !== null && (m.downloaded || downloadedIds.includes(m.id));
  const currentModelsDirError = $derived(
    dirInfo ? modelsDirPathError(dirInfo.dir) : null,
  );
  const primaryReady = $derived(
    isDownloaded(primaryModel) && currentModelsDirError === null,
  );

  let selectedProfileId = $state("lol");
  let selectedDeviceId = $state("default");
  let selectedMode = $state<InteractionMode>("push-to-talk");
  let currentKey = $state("CapsLock");

  const modes: { id: InteractionMode; name: string; desc: string; icon: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送", icon: "🎙️" },
    { id: "dictation", name: "录音笔", desc: "按一下开始，再按停止，确认后发送", icon: "⏺️" },
    { id: "one-shot", name: "说一句就走", desc: "按一下，说完自动发送", icon: "🚀" },
  ];

  const selectedProfile = $derived(
    profiles.find((p) => p.id === selectedProfileId) ?? null,
  );

  onMount(async () => {
    try {
      [models, profiles, devices] = await Promise.all([
        listModels(),
        listProfiles(),
        listAudioDevices(),
      ]);
      const settings = $settingsStore;
      const lolAvailable = profiles.some((p) => p.id === "lol");
      const activeProfileAvailable =
        settings?.activeProfileId && profiles.some((p) => p.id === settings.activeProfileId);
      if (!settings?.ui.firstRunCompleted && lolAvailable) {
        selectedProfileId = "lol";
      } else if (activeProfileAvailable) {
        selectedProfileId = settings.activeProfileId!;
      } else {
        selectedProfileId = lolAvailable ? "lol" : (profiles[0]?.id ?? "generic");
      }
      selectedDeviceId = settings?.audioDeviceId ?? devices[0]?.id ?? "default";
      selectedMode = settings?.interactionMode ?? "push-to-talk";
      currentKey = settings?.hotkey.key ?? "CapsLock";
      getModelsDir()
        .then((d) => (dirInfo = d))
        .catch((e) => console.error("[onboarding] get_models_dir 失败：", e));
    } catch (e) {
      resourceError = errText(e);
    } finally {
      loadingResources = false;
    }
  });

  /** 选择自定义模型存储目录（下载前可先迁移，避免模型落到 C 盘） */
  async function pickModelsDir() {
    if (changingDir) return;
    let picked: string | null = null;
    if (isTauri) {
      const sel = await openDialog({ directory: true, title: "选择模型存储位置" }).catch(
        () => null,
      );
      if (!sel) return; // 用户取消
      picked = sel;
    } else {
      picked = window.prompt("模型存储目录（开发预览模式）", dirInfo?.dir ?? "");
      if (!picked) return;
    }
    const pathError = modelsDirPathError(picked);
    if (pathError) {
      toast(false, pathError);
      return;
    }
    changingDir = true;
    try {
      const report = await setModelsDir(picked);
      if (report.failed.length > 0) {
        toastWarn(
          `目录已切换，但 ${report.failed.length} 项迁移失败（需重新下载）：${report.failed.join(", ")}`,
        );
      } else {
        toast(
          true,
          report.moved.length > 0 ? `模型目录已切换，迁移 ${report.moved.length} 项` : "模型目录已切换",
        );
      }
      dirInfo = await getModelsDir();
    } catch (e) {
      toast(false, `切换模型目录失败：${errText(e)}`);
    } finally {
      changingDir = false;
    }
  }

  /** 恢复默认目录（~/.kotone/models） */
  async function resetModelsDir() {
    if (changingDir) return;
    changingDir = true;
    try {
      const report = await setModelsDir("");
      toast(
        true,
        report.moved.length > 0 ? `已恢复默认目录，迁移 ${report.moved.length} 项` : "已恢复默认目录",
      );
      dirInfo = await getModelsDir();
    } catch (e) {
      toast(false, `恢复默认目录失败：${errText(e)}`);
    } finally {
      changingDir = false;
    }
  }

  async function reloadResources() {
    loadingResources = true;
    resourceError = "";
    try {
      [models, profiles, devices] = await Promise.all([
        listModels(),
        listProfiles(),
        listAudioDevices(),
      ]);
    } catch (e) {
      resourceError = errText(e);
    } finally {
      loadingResources = false;
    }
  }

  async function chooseProfile(id: string) {
    selectedProfileId = id;
    try {
      settingsStore.set(await updateSettings({ activeProfileId: id }));
      toast(true, `已选择 ${profiles.find((p) => p.id === id)?.displayName ?? id}`);
    } catch (e) {
      toast(false, `保存游戏配置失败：${errText(e)}`);
    }
  }

  async function chooseDevice(id: string) {
    selectedDeviceId = id;
    try {
      await setAudioDevice(id);
      settingsStore.update((s) => (s ? { ...s, audioDeviceId: id } : s));
      toast(true, "麦克风已切换");
    } catch (e) {
      toast(false, `切换麦克风失败：${errText(e)}`);
    }
  }

  let downloadTargetId = $state<string | null>(null);
  let dlProgress = $state<{ downloaded: number; total: number } | null>(null);
  let dlError = $state("");
  let unlistenDl: (() => void) | undefined;
  let cancellingDownload = $state(false);
  let manualGuide = $state<ModelInstallGuide | null>(null);

  const dlPercent = $derived(
    dlProgress && dlProgress.total > 0
      ? Math.min(100, Math.round((dlProgress.downloaded / dlProgress.total) * 100))
      : null,
  );

  async function downloadById(id: string) {
    // 下载目录必须先加载完成，且目录迁移和模型下载不能并发进行。
    if (downloadTargetId || changingDir || dirInfo === null) return;
    if (currentModelsDirError) {
      toast(false, currentModelsDirError);
      return;
    }
    downloadTargetId = id;
    dlProgress = null;
    dlError = "";
    try {
      if (isTauri) {
        unlistenDl = await listen<DownloadProgress>("kotone://download", (ev) => {
          if (ev.payload.id === id) {
            dlProgress = {
              downloaded: ev.payload.downloaded,
              total: ev.payload.total,
            };
          }
        });
      }
      await downloadModel(id);
      if (!downloadedIds.includes(id)) downloadedIds = [...downloadedIds, id];
      models = models.map((m) => (m.id === id ? { ...m, downloaded: true } : m));
      toast(true, "模型下载完成，可以继续");
    } catch (e) {
      if (isDownloadCancelled(e)) {
        toastInfo("已取消下载，可随时继续");
      } else {
        dlError = errText(e);
        toast(false, `模型下载失败：${dlError}`);
        await openManualGuide(id);
      }
    } finally {
      downloadTargetId = null;
      cancellingDownload = false;
      unlistenDl?.();
      unlistenDl = undefined;
    }
  }

  /** 取消下载（.part 保留，可续传；P2-⑦） */
  async function cancelDownloadById() {
    if (cancellingDownload) return;
    cancellingDownload = true;
    try {
      await cancelDownload();
    } catch (e) {
      cancellingDownload = false;
      toast(false, `取消失败：${errText(e)}`);
    }
  }

  async function openManualGuide(id: string) {
    try {
      manualGuide = await getModelInstallGuide(id);
    } catch (e) {
      toast(false, `无法加载手动安装指引：${errText(e)}`);
    }
  }

  async function recheckAfterManualInstall() {
    const id = manualGuide?.id;
    try {
      models = await listModels();
    } catch (e) {
      toast(false, `重新检测失败：${errText(e)}`);
      return;
    }
    if (id && isDownloaded(models.find((m) => m.id === id) ?? null)) {
      dlError = "";
      manualGuide = null;
      toast(true, "已检测到模型文件");
    } else {
      toastWarn("还没检测到完整模型，请确认文件名和目录后重试");
    }
  }

  let capturing = $state(false);
  let captureCleanup: (() => void) | null = null;
  let checkingInputEnvironment = $state(false);
  let inputEnvironment = $state<InputEnvironmentCheck | null>(null);
  /** 拦截逃生门：探针判定拦截时用户仍可选择「仍然继续」（风险自担），不被卡在向导里 */
  let inputBlockedOverridden = $state(false);

  async function runInputEnvironmentCheck() {
    if (checkingInputEnvironment) return;
    checkingInputEnvironment = true;
    try {
      inputEnvironment = await checkInputEnvironment();
      if (!inputEnvironment.available) {
        toast(
          false,
          "检测到键盘钩子或模拟输入被系统拦截，请先将 Kotone 加入 360、火绒等安全软件的信任区。",
        );
      }
    } catch (e) {
      inputEnvironment = {
        available: false,
        hookVerified: false,
        observed: 0,
        expected: 0,
        detail: errText(e),
      };
      toast(false, `输入环境检测失败：${errText(e)}`);
    } finally {
      checkingInputEnvironment = false;
    }
  }

  function enterHotkeyStep() {
    step = 3;
    // 第三步一出现就主动检测，不要求用户先点击「重新录入」或启动运行时。
    void runInputEnvironmentCheck();
  }

  async function startCapture() {
    if (
      capturing ||
      checkingInputEnvironment ||
      (inputEnvironment?.available === false && !inputBlockedOverridden)
    ) return;
    capturing = true;
    captureCleanup = await captureHotkey((result) => {
      capturing = false;
      captureCleanup = null;
      if (result.kind === "combo") {
        currentKey = result.combo;
        toast(true, `已录入热键：${result.combo}`);
        void checkHotkeyConflicts(result.combo);
      } else if (result.kind === "cancelled") {
        toastInfo("已取消录入");
      } else if (result.kind === "timeout") {
        toastWarn("录入超时，请重试");
      } else {
        toast(false, result.message);
      }
    });
  }

  /** 键位冲突静态提示（P2-⑩：录入后展示常见游戏键位冲突） */
  let hotkeyWarnings = $state<string[]>([]);
  async function checkHotkeyConflicts(key: string) {
    try {
      hotkeyWarnings = await detectHotkeyConflicts(key);
    } catch {
      hotkeyWarnings = [];
    }
  }

  async function saveInteractionAndContinue() {
    if (finishing) return;
    finishing = true;
    try {
      const mode = selectedMode === "push-to-talk" ? "hold" : "toggle";
      settingsStore.set(
        await updateSettings({
          activeProfileId: selectedProfileId,
          interactionMode: selectedMode,
          hotkey: { key: currentKey, mode },
          ui: { firstRunCompleted: true },
        }),
      );
    } catch (e) {
      toast(false, `保存说话方式失败：${errText(e)}`);
      finishing = false;
      return;
    }
    onDone();
  }


  let finishing = $state(false);
  async function finish(skipped = false) {
    if (finishing) return;
    finishing = true;
    try {
      const patchObj: Record<string, unknown> = {
        activeProfileId: selectedProfileId,
        ui: { firstRunCompleted: true },
      };
      if (skipped) {
        // 跳过向导也要留下明确的交互模式（默认对讲机），避免「无模式」空状态
        patchObj.interactionMode = selectedMode;
        patchObj.hotkey = {
          key: currentKey,
          mode: selectedMode === "push-to-talk" ? "hold" : "toggle",
        };
      }
      settingsStore.set(await updateSettings(patchObj));
    } catch (e) {
      toast(false, `保存向导状态失败：${errText(e)}`);
      finishing = false;
      return;
    }
    onDone();
  }

  onDestroy(() => {
    captureCleanup?.();
    unlistenDl?.();
  });
</script>

<div
  class="absolute inset-0 z-50 flex items-center justify-center bg-kotone-deep/94 p-5 backdrop-blur-sm"
  data-testid="onboarding"
>
  <div
    class="kotone-panel relative max-h-full w-full max-w-2xl overflow-y-auto shadow-glow-cyan-lg"
  >
    <div
      class="pointer-events-none absolute -top-24 left-1/2 h-48 w-96 -translate-x-1/2 rounded-full bg-kotone-violet/25 blur-3xl"
    ></div>

    {#if step === 0}
      <div class="relative overflow-hidden">
        <!-- 欢迎视觉：ord-ui-onboarding-hero 合成图（张开双臂的邀请感） -->
        <div class="relative">
          <img src={heroOnboarding} alt="Kotone 张开双臂欢迎来到直播间" class="h-52 w-full object-cover object-top" />
          <div class="absolute inset-0 bg-gradient-to-t from-kotone-panel via-kotone-panel/30 to-transparent"></div>
        </div>
        <div class="relative -mt-8 px-8 pb-8">
          <img src={stickerHello} alt="" class="mb-3 h-12 w-12" />
          <h1 class="text-2xl font-bold">
            欢迎来到 <span class="kotone-gradient-text">Kotone 琴音</span>
          </h1>
          <p class="mt-2 text-sm leading-relaxed text-white/65">
            游戏里不动手，说话就能发消息。<br />
            接下来会选游戏配置、准备本地模型、设置热键，完成后直接进入主页。
          </p>
          <div class="mt-6 flex items-center gap-3">
            <button
              class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 focus-visible:ring-2 focus-visible:ring-white active:scale-95"
              onclick={() => (step = 1)}
            >
              开始设置
            </button>
            <button
              class="text-xs text-white/50 underline-offset-2 transition hover:text-white/80 hover:underline"
              onclick={() => void finish(true)}
            >
              跳过向导
            </button>
          </div>
        </div>
      </div>
    {:else if step === 1}
      <div class="relative p-7" data-testid="onboarding-profile">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 1 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">选择游戏配置</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              Profile 只决定聊天键、输入方式和术语词库，不会限制你向哪个前台窗口发送。
            </p>
          </div>
        </div>

        {#if loadingResources}
          <p class="mt-5 text-sm text-white/55">正在读取游戏配置…</p>
        {:else if resourceError}
          <div class="mt-5 rounded-lg bg-kotone-pink/10 p-3 ring-1 ring-kotone-pink/40">
            <p class="text-xs text-kotone-pink">{resourceError}</p>
            <button class="mt-2 text-xs text-white underline" onclick={() => void reloadResources()}>
              重试
            </button>
          </div>
        {:else}
          <div class="mt-5 grid grid-cols-2 gap-3">
            {#each profiles as profile}
              {@const active = selectedProfileId === profile.id}
              <button
                class="relative rounded-xl p-4 text-left ring-1 transition focus-visible:ring-2 focus-visible:ring-kotone-cyan {active
                  ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan'
                  : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
                data-testid="profile-{profile.id}"
                onclick={() => void chooseProfile(profile.id)}
              >
                {#if profile.id === "lol"}
                  <span class="absolute top-3 right-3 rounded bg-kotone-pink/20 px-2 py-0.5 text-[10px] font-semibold text-kotone-pink">
                    推荐
                  </span>
                {/if}
                <span class="text-xl">{profile.id === "lol" ? "🎮" : "🌐"}</span>
                <p class="mt-2 text-sm font-semibold {active ? 'text-kotone-cyan' : ''}">
                  {profile.displayName}
                </p>
                <p class="mt-1 text-[11px] leading-relaxed text-white/50">
                  {profile.openChatKey} 打开聊天 · {profile.sendKey} 发送 · 热词 {profile.hotwords.length} 个
                </p>
              </button>
            {/each}
          </div>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 0)}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-50"
            disabled={loadingResources || !selectedProfile}
            onclick={() => (step = 2)}
          >
            下一步
          </button>
        </div>
      </div>
    {:else if step === 2}
      <div class="relative p-6" data-testid="onboarding-model">
        <div class="flex items-start gap-4">
          <img src={stickerThinking} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 2 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">准备识别模型与麦克风</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              识别完全在本地运行。推荐模型只需下载一次，不会上传你的语音。
            </p>
          </div>
        </div>

        <!-- 先确认存储位置，再下载模型；两项属于同一准备流程。 -->
        <div class="kotone-card mt-4 overflow-hidden" data-testid="model-setup-section">
          <div class="border-b border-white/10 p-3.5" data-testid="model-storage-section">
            <div class="flex items-center gap-3">
              <span class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-kotone-cyan/15 text-[11px] font-bold text-kotone-cyan">
                1
              </span>
              <div class="min-w-0 flex-1">
                <p class="text-sm font-semibold">先确认模型存储位置</p>
                <p
                  class="mt-0.5 truncate text-[11px] text-white/45"
                  title={dirInfo?.dir ?? "正在读取模型存储位置"}
                  data-testid="models-dir-path"
                >
                  {dirInfo?.dir ?? "正在读取…"}{dirInfo?.isDefault ? "（默认）" : ""}
                </p>
              </div>
              <div class="flex shrink-0 items-center gap-1.5">
                {#if dirInfo && !dirInfo.isDefault}
                  <button
                    class="rounded-lg bg-white/8 px-2.5 py-1.5 text-[11px] text-white/65 ring-1 ring-white/12 transition hover:bg-white/15 disabled:opacity-50"
                    disabled={changingDir || downloadTargetId !== null}
                    onclick={() => void resetModelsDir()}
                  >
                    恢复默认
                  </button>
                {/if}
                <button
                  class="rounded-lg bg-white/10 px-2.5 py-1.5 text-[11px] font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20 disabled:opacity-50"
                  disabled={changingDir || downloadTargetId !== null || dirInfo === null}
                  onclick={() => void pickModelsDir()}
                >
                  {changingDir ? "切换中…" : "选择位置"}
                </button>
                <button
                  class="rounded-lg bg-white/8 px-2.5 py-1.5 text-[11px] text-white/65 ring-1 ring-white/12 transition hover:bg-white/15 disabled:opacity-50"
                  disabled={dirInfo === null}
                  onclick={() => void openModelsDir()}
                >
                  打开
                </button>
              </div>
            </div>
            {#if currentModelsDirError}
              <p class="mt-2 text-[11px] text-kotone-pink" data-testid="models-dir-error">
                {currentModelsDirError}。请先点击“选择位置”。
              </p>
            {:else}
              <p class="mt-2 text-[11px] text-white/40">
                模型路径需要使用纯英文，例如 D:\KotoneModels。
              </p>
            {/if}
          </div>

          <div class="p-3.5" data-testid="model-download-section">
            <div class="flex items-center gap-3">
              <span class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-kotone-violet/18 text-[11px] font-bold text-kotone-violet">
                2
              </span>
              {#if primaryModel}
                <div class="min-w-0 flex-1">
                  <p class="truncate text-sm font-semibold">再下载 {primaryModel.displayName}</p>
                  <p class="mt-0.5 text-[11px] text-white/45">
                    约 {Math.round(primaryModel.sizeBytes / 1_000_000)} MB · 中英流式识别
                  </p>
                </div>
                {#if primaryReady}
                  <span class="shrink-0 rounded bg-kotone-cyan/15 px-3 py-1.5 text-xs font-semibold text-kotone-cyan">
                    ✓ 已就绪
                  </span>
                {:else}
                  <button
                    class="shrink-0 rounded-lg bg-kotone-cyan px-3.5 py-1.5 text-xs font-semibold text-kotone-deep disabled:opacity-50"
                    disabled={downloadTargetId !== null || changingDir || dirInfo === null || currentModelsDirError !== null}
                    onclick={() => void downloadById(primaryModel.id)}
                  >
                    {downloadTargetId === primaryModel.id ? "下载中…" : dlError ? "重试下载" : "下载推荐模型"}
                  </button>
                {/if}
              {:else}
                <p class="text-xs text-kotone-pink">未找到推荐模型，请重试读取资源。</p>
              {/if}
            </div>
            {#if primaryModel && downloadTargetId === primaryModel.id}
              <div class="mt-2.5 flex items-center gap-2 pl-9">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-white/10">
                  <div
                    class="h-full rounded-full bg-kotone-cyan transition-[width] {dlPercent === null
                      ? 'w-1/3 animate-pulse'
                      : ''}"
                    style:width={dlPercent === null ? undefined : `${dlPercent}%`}
                  ></div>
                </div>
                <span class="w-9 text-right text-[10px] text-white/45">
                  {dlPercent === null ? "连接" : `${dlPercent}%`}
                </span>
                <button
                  class="shrink-0 rounded-lg bg-white/10 px-2.5 py-1 text-[11px] text-white/70 ring-1 ring-white/15 transition hover:bg-white/20 disabled:opacity-50"
                  disabled={cancellingDownload}
                  onclick={() => void cancelDownloadById()}
                >
                  {cancellingDownload ? "正在取消…" : "取消"}
                </button>
              </div>
            {/if}
            {#if dlError}
              <div class="mt-2 flex items-start gap-2 pl-9">
                <p class="min-w-0 flex-1 text-[11px] text-kotone-pink">{dlError}</p>
                {#if primaryModel}
                  <button
                    class="shrink-0 text-[11px] text-white/60 underline-offset-2 hover:text-white/85 hover:underline"
                    onclick={() => void openManualGuide(primaryModel.id)}
                  >
                    手动下载
                  </button>
                {/if}
              </div>
            {/if}
          </div>
        </div>

        <label class="kotone-card mt-3 flex items-center gap-3 p-3">
          <span class="shrink-0 text-sm font-semibold text-kotone-cyan/90">麦克风</span>
          <select
            class="min-w-0 flex-1 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
            value={selectedDeviceId}
            aria-label="麦克风"
            onchange={(e) => void chooseDevice((e.target as HTMLSelectElement).value)}
          >
            {#each devices as device}
              <option value={device.id}>{device.name}</option>
            {/each}
          </select>
          <span class="shrink-0 text-[10px] text-white/40">下一步会验证</span>
        </label>

        <div class="mt-4 flex items-center justify-between" data-testid="onboarding-model-footer">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 1)}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-50"
            disabled={!primaryReady || downloadTargetId !== null}
            onclick={enterHotkeyStep}
          >
            下一步
          </button>
        </div>
      </div>
    {:else if step === 3}
      <div class="relative p-7" data-testid="onboarding-hotkey">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 3 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">设置说话方式与热键</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              选择最顺手的操作方式，点「完成」进入主页后即可开始使用。
            </p>
          </div>
        </div>

        <div class="mt-5 grid grid-cols-3 gap-2">
          {#each modes as mode}
            {@const active = selectedMode === mode.id}
            <button
              class="rounded-xl px-3 py-3 text-left ring-1 transition focus-visible:ring-2 focus-visible:ring-kotone-cyan {active
                ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan'
                : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
              data-testid="mode-{mode.id}"
              onclick={() => (selectedMode = mode.id)}
            >
              <span class="text-lg">{mode.icon}</span>
              <p class="mt-1 text-sm font-semibold {active ? 'text-kotone-cyan' : ''}">{mode.name}</p>
              <p class="mt-1 text-[11px] leading-relaxed text-white/50">{mode.desc}</p>
            </button>
          {/each}
        </div>

        {#if checkingInputEnvironment}
          <div
            class="mt-4 rounded-xl bg-kotone-cyan/8 p-4 ring-1 ring-kotone-cyan/30"
            data-testid="input-environment-checking"
          >
            <p class="text-sm font-semibold text-kotone-cyan">正在检测键盘输入环境…</p>
            <p class="mt-1 text-[11px] leading-relaxed text-white/50">
              正在验证低级键盘钩子与 Windows 模拟输入，无需按下任何按键。
            </p>
          </div>
        {:else if inputEnvironment && !inputEnvironment.available}
          <div
            class="mt-4 rounded-xl bg-kotone-pink/10 p-4 ring-1 ring-kotone-pink/45"
            data-testid="input-environment-blocked"
          >
            <p class="text-sm font-semibold text-kotone-pink">检测到键盘输入被拦截</p>
            <p class="mt-1 text-xs leading-relaxed text-white/65">
              Kotone 暂时无法可靠监听热键或发送文字。请将 Kotone 可执行文件加入
              360、火绒等安全软件的信任区，然后重新检测。
            </p>
            {#if inputEnvironment.detail}
              <p class="mt-2 break-all text-[10px] leading-relaxed text-white/35">
                检测详情：{inputEnvironment.detail}
              </p>
            {/if}
            <button
              class="mt-3 rounded-lg bg-kotone-pink/18 px-3 py-1.5 text-xs font-semibold text-kotone-pink ring-1 ring-kotone-pink/35 hover:bg-kotone-pink/25"
              onclick={() => void runInputEnvironmentCheck()}
            >
              已加入信任区，重新检测
            </button>
            <button
              class="mt-3 rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/70 ring-1 ring-white/15 transition hover:bg-white/20"
              onclick={() => {
                inputBlockedOverridden = true;
                toastWarn("已跳过输入环境检测：如后续发送无反应，请回此步重新检测");
              }}
            >
              仍然继续（风险自担）
            </button>
          </div>
        {:else if inputEnvironment}
          <div
            class="mt-4 rounded-xl bg-kotone-cyan/8 p-3 ring-1 ring-kotone-cyan/25"
            data-testid="input-environment-ready"
          >
            <p class="text-xs font-semibold text-kotone-cyan">✓ 键盘输入环境可用</p>
            {#if !inputEnvironment.hookVerified}
              <p class="mt-1 text-[10px] leading-relaxed text-white/40">
                Windows 已接受模拟输入；低级钩子将在你实际录入按键时继续验证。
              </p>
            {/if}
          </div>
        {/if}

        <div class="kotone-card mt-4 flex items-center gap-3 p-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs text-white/50">当前热键</p>
            <p class="mt-1 text-lg font-bold text-kotone-cyan">{currentKey}</p>
          </div>
          <button
            class="rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/85 ring-1 ring-white/15 hover:bg-white/20 disabled:opacity-60"
            disabled={capturing || checkingInputEnvironment || (inputEnvironment?.available === false && !inputBlockedOverridden)}
            onclick={() => void startCapture()}
          >
            {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "重新录入"}
          </button>
        </div>

        {#if hotkeyWarnings.length > 0}
          <div class="mt-3 rounded-xl bg-kotone-pink/10 p-3 ring-1 ring-kotone-pink/35">
            <p class="text-xs font-semibold text-kotone-pink">键位冲突提示</p>
            {#each hotkeyWarnings as w}
              <p class="mt-1 text-[11px] leading-relaxed text-white/65">⚠ {w}</p>
            {/each}
          </div>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 2)}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-50"
            disabled={capturing || checkingInputEnvironment || (inputEnvironment?.available === false && !inputBlockedOverridden)}
            onclick={() => void saveInteractionAndContinue()}
          >
            完成
          </button>
        </div>
      </div>
    {/if}

    <div class="relative flex justify-center gap-1.5 pb-4">
      {#each [0, 1, 2, 3] as i}
        <span
          class="h-1.5 rounded-full transition-all {i === step
            ? 'w-5 bg-kotone-cyan shadow-glow-cyan'
            : 'w-1.5 bg-white/20'}"
        ></span>
      {/each}
    </div>
  </div>
</div>

{#if manualGuide}
  <ManualDownloadDialog
    guide={manualGuide}
    error={dlError}
    onClose={() => (manualGuide = null)}
    onRetry={() => {
      const id = manualGuide?.id;
      manualGuide = null;
      if (id) void downloadById(id);
    }}
    onRecheck={() => void recheckAfterManualInstall()}
  />
{/if}
