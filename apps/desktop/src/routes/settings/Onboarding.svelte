<script lang="ts">
  import { companionName } from "$lib/stores/companion";
  import { onDestroy, onMount, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    checkInputEnvironment,
    cancelDownload,
    detectHotkeyConflicts,
    downloadModel,
    getModelsDir,
    getProfileIcon,
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
    getSettings,
    upsertConnection,
    deleteConnection,
    connectionSelfCheck,
    companionBindings,
    setActiveModel,
    activateCatalogModel,
    startRuntime,
    openExternal,
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
  import { findHotkeyConflict, hotkeyLabel as formatHotkey } from "../../lib/hotkeyCombo";
  import { subscribeAgent } from "../../lib/agent";
  import { appState } from "../../lib/stores/state";
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
  import SelectField from "$lib/components/settings/SelectField.svelte";

  /** next = "assistant"：结束后进「琴音」的聊天；prompt：顺便替玩家发这句话。 */
  let { onDone, legacyUpgrade = false, onCancel }: {
    onDone: (next?: "assistant", prompt?: string) => void;
    legacyUpgrade?: boolean;
    onCancel?: () => void;
  } = $props();

  // 两条路（只告诉玩家怎么开始，带他完成第一次）：
  // - 和琴音开始：填一个 DeepSeek Key → 第一次“嗨琴音，你能干什么？”
  // - 只用语音输入法：选游戏 → 识别模型 → 热键 → 第一次说话上屏
  type Step = "welcome" | "key" | "call-key" | "hello" | "profile" | "model" | "hotkey" | "try";
  let step = $state<Step>(untrack(() => legacyUpgrade ? "key" : "welcome"));
  let path = $state<"ai" | "voice">("ai");
  const dots = $derived<Step[]>(legacyUpgrade ? ["key", "call-key", "hello"]
    : path === "ai" ? ["welcome", "key", "hello"] : ["welcome", "profile", "model", "hotkey", "try"]);
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
  let iconUrls = $state<Record<string, string>>({});
  const loadingIcons = $state<Set<string>>(new Set());

  const modes: { id: InteractionMode; name: string; desc: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送" },
    { id: "dictation", name: "录音笔", desc: "按一下开始，再按停止，确认后发送" },
    { id: "one-shot", name: "说一句就走", desc: "按一下，说完自动发送" },
  ];

  /** 加载 profile 图标字节 → data URL（按扩展名推断 mime；无图标/失败 → 占位） */
  async function ensureIcon(p: GameProfile) {
    if (!p.icon || iconUrls[p.id] !== undefined || loadingIcons.has(p.id)) return;
    loadingIcons.add(p.id);
    try {
      const bytes = await getProfileIcon(p.id);
      if (bytes.length === 0) {
        iconUrls[p.id] = "";
        return;
      }
      const ext = (p.icon.split(".").pop() ?? "webp").toLowerCase();
      const mime =
        ext === "png" ? "image/png" : ext === "jpg" || ext === "jpeg" ? "image/jpeg" : "image/webp";
      iconUrls[p.id] = `data:${mime};base64,${bytesToBase64(bytes)}`;
    } catch {
      iconUrls[p.id] = "";
    } finally {
      loadingIcons.delete(p.id);
    }
  }

  function bytesToBase64(bytes: Uint8Array): string {
    let binary = "";
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }

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
      for (const p of profiles) void ensureIcon(p);
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
      for (const p of profiles) void ensureIcon(p);
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
    step = "hotkey";
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
    const conflict = findHotkeyConflict($settingsStore, "record", currentKey);
    if (conflict) { toast(false, `录制热键与${conflict}冲突，请重新录入`); return; }
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
    finishing = false;
    step = "try";
    void startTry();
  }

  // ---------- 和琴音开始：一个 DeepSeek Key ----------
  const DEEPSEEK_KEY_PAGE = "https://platform.deepseek.com/api_keys";
  const hello = $derived(`嗨${$companionName}，你能干什么？`);
  const configuredCallKey = $derived(companionBindings($settingsStore?.assistant)?.hotkey);
  const callKey = $derived(configuredCallKey ? formatHotkey(configuredCallKey) : "~");
  let apiKey = $state("");
  let keyBusy = $state(false);
  let keyError = $state("");
  let keyConnectionId = "";
  let callKeyDraft = $state("");
  let callKeyError = $state("");
  let activatingLegacy = $state(false);
  let tentativeConnectionId = "";

  async function removeTentativeConnection() {
    if (!tentativeConnectionId) return;
    settingsStore.set(await deleteConnection(tentativeConnectionId));
    tentativeConnectionId = "";
  }

  async function cancelLegacy() {
    if (activatingLegacy) return;
    activatingLegacy = true;
    try {
      await removeTentativeConnection();
      apiKey = "";
      onCancel?.();
    } catch (error) {
      if (step === "key") keyError = errText(error);
      else callKeyError = errText(error);
    } finally {
      activatingLegacy = false;
    }
  }

  async function recordCallKey() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((result) => {
      capturing = false;
      captureCleanup = null;
      if (result.kind === "combo") {
        callKeyDraft = result.combo;
        callKeyError = "";
      } else if (result.kind === "error") {
        callKeyError = result.message;
      }
    });
  }

  async function activateLegacyAssistant() {
    const hotkey = callKeyDraft.trim();
    if (!hotkey || activatingLegacy) return;
    const conflict = findHotkeyConflict($settingsStore, "assistant", hotkey);
    if (conflict) {
      callKeyError = `这个键已经用于${conflict}，请换一个`;
      return;
    }
    activatingLegacy = true;
    callKeyError = "";
    let checkingKey = true;
    try {
      await removeTentativeConnection();
      const connectionId = `deepseek-${crypto.randomUUID()}`;
      await saveDeepSeekConnection(connectionId, apiKey.trim());
      tentativeConnectionId = connectionId;
      const check = await connectionSelfCheck(connectionId);
      if (!check.passed) throw new Error(check.message || "这个 Key 用不了，检查一下有没有复制完整");
      checkingKey = false;
      const next = await updateSettings({
        assistant: { companionBindings: { connectionId, hotkey } },
        ui: { legacyUpgrade: "optedIn", firstRunCompleted: true },
      });
      tentativeConnectionId = "";
      apiKey = "";
      settingsStore.set(next);
      void startRuntime().catch(() => {});
      step = "hello";
      void watchHello();
    } catch (error) {
      let message = errText(error);
      try {
        await removeTentativeConnection();
      } catch (cleanupError) {
        message += `。移除本次新建的连接失败：${errText(cleanupError)}`;
      }
      if (checkingKey) {
        keyError = message;
        step = "key";
      } else {
        callKeyError = message;
      }
    } finally {
      activatingLegacy = false;
    }
  }

  async function saveDeepSeekConnection(id: string, key: string) {
    await upsertConnection(
      { id, displayName: "DeepSeek", kind: "remote", provider: "deepseek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-flash" },
      key,
    );
  }

  async function startWithKey() {
    const key = apiKey.trim();
    if (!key || keyBusy || activatingLegacy) return;
    if (legacyUpgrade) {
      step = "call-key";
      return;
    }
    keyBusy = true;
    keyError = "";
    try {
      keyConnectionId ||= `deepseek-${Date.now().toString(36)}`;
      await saveDeepSeekConnection(keyConnectionId, key);
      // 读一次模型列表确认 Key 能用（不花钱）。
      const check = await connectionSelfCheck(keyConnectionId);
      if (!check.passed) {
        keyError = check.message || "这个 Key 用不了，检查一下有没有复制完整";
        return;
      }
      // 一个 Key 就能用全部功能：琴音、语音识别、读屏都走 DeepSeek（以后想换本地模型再下）。
      await updateSettings({ assistant: { connectionId: keyConnectionId }, activeProfileId: selectedProfileId, ui: { firstRunCompleted: true } });
      if (!models.some((m) => m.downloaded)) await setActiveModel("deepseek-asr", "deepseek-asr");
      if (!$settingsStore?.ocr?.activeModelId) await activateCatalogModel("ocr", "deepseek-ocr");
      settingsStore.set(await getSettings());
      void startRuntime().catch(() => {});
      step = "hello";
      void watchHello();
    } catch (e) {
      keyError = errText(e);
    } finally {
      keyBusy = false;
    }
  }

  /** 第一次打招呼：听她有没有开始想、有没有回答（按呼叫键说，或者在聊天里打字都算）。 */
  let helloState = $state<"waiting" | "thinking" | "answered">("waiting");
  let helloAnswer = $state("");
  let unlistenAgent: (() => void) | undefined;
  async function watchHello() {
    unlistenAgent?.();
    unlistenAgent = await subscribeAgent((_, event) => {
      if (event.kind === "step" && helloState === "waiting") helloState = "thinking";
      if (event.kind === "appended" && event.message.role === "assistant" && event.message.text.trim()) {
        helloState = "answered";
        helloAnswer = event.message.text.trim();
      }
    });
  }

  // ---------- 只用语音输入法：第一次说话上屏 ----------
  let tryText = $state("");
  let tryBox = $state<HTMLTextAreaElement | null>(null);
  const tried = $derived(tryText.trim().length > 0 || $appState.state === "success");
  async function startTry() {
    try {
      await startRuntime();
    } catch (e) {
      toast(false, `启动语音输入失败：${errText(e)}`);
    }
    tryBox?.focus();
  }


  let finishing = $state(false);
  async function finish(skipped = false) {
    if (finishing) return;
    const conflict = skipped ? findHotkeyConflict($settingsStore, "record", currentKey) : null;
    if (conflict) { toast(false, `录制热键与${conflict}冲突，请重新录入`); return; }
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
    unlistenAgent?.();
  });
</script>

<div
  class="absolute inset-0 z-50 flex items-center justify-center bg-black/50 p-5 backdrop-blur-sm"
  data-testid="onboarding"
>
  <div
    class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl relative max-h-full w-full max-w-2xl overflow-y-auto"
  >
    <div
      class="pointer-events-none absolute -top-24 left-1/2 h-48 w-96 -translate-x-1/2 rounded-full bg-kotone-violet/25 blur-3xl"
    ></div>

    {#if step === "welcome"}
      <div class="relative overflow-hidden">
        <!-- 欢迎视觉：ord-ui-onboarding-hero 合成图（张开双臂的邀请感） -->
        <div class="relative">
          <img src={heroOnboarding} alt="Kotone 张开双臂欢迎来到直播间" class="h-52 w-full object-cover object-top" />
          <div class="absolute inset-0 bg-gradient-to-t from-popover via-popover/30 to-transparent"></div>
        </div>
        <div class="relative -mt-8 px-8 pb-8">
          <img src={stickerHello} alt="" class="mb-3 h-12 w-12" />
          <h1 class="text-2xl font-bold">
            欢迎来到 <span class="kotone-gradient-text">Kotone 琴音</span>
          </h1>
          <p class="mt-2 text-sm leading-relaxed text-muted-foreground">陪你打游戏的 AI：叫她一声，她帮你查、帮你说、帮你按。</p>
          <div class="mt-6 flex flex-wrap items-center gap-3">
            <button
              class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 focus-visible:ring-2 focus-visible:ring-ring active:scale-95"
              data-testid="onboarding-start-ai"
              onclick={() => { path = "ai"; step = "key"; }}
            >
              和{$companionName}开始
            </button>
            <button
              class="rounded-lg border border-hairline px-4 py-2 text-sm text-foreground/85 transition hover:bg-muted"
              data-testid="onboarding-start-voice"
              onclick={() => { path = "voice"; step = "profile"; }}
            >
              只用语音输入法
            </button>
            <button
              class="ml-auto text-xs text-muted-foreground underline-offset-2 transition hover:text-foreground/90 hover:underline"
              onclick={() => void finish(true)}
            >
              跳过
            </button>
          </div>
        </div>
      </div>
    {:else if step === "key"}
      <div class="relative p-7" data-testid="onboarding-key">
        <div class="flex items-start gap-4">
          <img src={stickerHello} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <h2 class="mt-0.5 text-lg font-bold">填一个 DeepSeek Key</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              {legacyUpgrade ? "这个 Key 用于和琴音对话。你原来的语音识别和游戏配置会保留。" : "有这一个 Key，" + $companionName + "的全部功能就能用。"}
            </p>
          </div>
        </div>
        <form class="mt-5" onsubmit={(e) => { e.preventDefault(); void startWithKey(); }}>
          <input
            type="password"
            autocomplete="off"
            spellcheck="false"
            placeholder="sk-…"
            aria-label="DeepSeek Key"
            data-testid="onboarding-key-input"
            bind:value={apiKey}
            class="h-10 w-full rounded-lg border border-hairline bg-background px-3 font-mono text-sm outline-none focus:border-primary"
          />
          {#if keyError}<p role="alert" class="mt-2 mb-0 text-xs text-destructive" data-testid="onboarding-key-error">{keyError}</p>{/if}
          <button type="button" class="mt-2 text-xs text-primary hover:underline" onclick={() => void openExternal(DEEPSEEK_KEY_PAGE).catch(() => {})}>
            还没有 Key？去 DeepSeek 开放平台创建 →
          </button>
          <div class="mt-6 flex items-center justify-between">
            <button type="button" class="text-xs text-muted-foreground hover:text-foreground/90" disabled={keyBusy || activatingLegacy}
              onclick={() => { if (legacyUpgrade) void cancelLegacy(); else step = "welcome"; }}>返回</button>
            <button
              type="submit"
              class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
              data-testid="onboarding-key-start"
              disabled={!apiKey.trim() || keyBusy || activatingLegacy}
            >
              {keyBusy ? "连接中…" : legacyUpgrade ? "下一步" : "开始"}
            </button>
          </div>
        </form>
      </div>
    {:else if step === "call-key"}
      <div class="relative p-7" data-testid="onboarding-call-key">
        <h2 class="mt-0.5 text-lg font-bold">设置呼叫琴音的按键</h2>
        <p class="mt-1 text-xs leading-relaxed text-muted-foreground">请选一个游戏里用不到的键。有些游戏用 ~ 打开控制台，绑定相同按键会冲突。</p>
        <form class="mt-5" onsubmit={(event) => { event.preventDefault(); void activateLegacyAssistant(); }}>
          <div class="flex flex-wrap items-center gap-2">
            <input aria-label="呼叫热键" data-testid="onboarding-call-key-input" bind:value={callKeyDraft}
              placeholder="如 Ctrl+Alt+F10" spellcheck="false" disabled={capturing || activatingLegacy}
              class="h-10 min-w-0 flex-1 rounded-lg border border-hairline bg-background px-3 font-mono text-sm outline-none focus:border-primary" />
            <button type="button" class="rounded-lg border border-hairline px-3 py-2 text-xs"
              disabled={capturing || activatingLegacy} onclick={() => void recordCallKey()}>
              {capturing ? "请按键…" : "录入按键"}
            </button>
          </div>
          {#if callKeyError}<p role="alert" class="mt-2 text-xs text-destructive">{callKeyError}</p>{/if}
          <div class="mt-6 flex items-center justify-between">
            <button type="button" class="text-xs text-muted-foreground hover:text-foreground/90" disabled={activatingLegacy}
              onclick={() => void cancelLegacy()}>先不启用</button>
            <button type="submit" data-testid="onboarding-call-key-enable"
              class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground disabled:opacity-50"
              disabled={!callKeyDraft.trim() || capturing || activatingLegacy}>{activatingLegacy ? "正在开启…" : "开启琴音"}</button>
          </div>
        </form>
      </div>
    {:else if step === "hello"}
      <div class="relative p-7" data-testid="onboarding-hello">
        <div class="flex items-start gap-4">
          <img src={helloState === "answered" ? stickerCheering : stickerHello} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <h2 class="mt-0.5 text-lg font-bold">跟她打个招呼</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">在游戏里也一样：按一下就叫她。</p>
          </div>
        </div>
        <div class="mt-5 rounded-2xl bg-muted/40 px-5 py-4 ring-1 ring-hairline">
          <p class="m-0 text-sm text-muted-foreground">按一下 <kbd class="rounded border border-hairline bg-background px-1.5 py-0.5 font-mono text-xs text-foreground">{callKey}</kbd>，说：</p>
          <p class="m-0 mt-1.5 text-lg font-semibold">“{hello}”</p>
        </div>
        <p class="mt-3 mb-0 min-h-5 text-xs" data-testid="onboarding-hello-status" role="status">
          {#if helloState === "answered"}
            <span class="text-success">✓ 她回答了：</span><span class="text-foreground/85">{helloAnswer.length > 80 ? `${helloAnswer.slice(0, 80)}…` : helloAnswer}</span>
          {:else if helloState === "thinking"}
            <span class="text-primary">她在想…</span>
          {:else}
            <span class="text-muted-foreground">等你开口…</span>
          {/if}
        </p>
        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-muted-foreground hover:text-foreground/90" data-testid="onboarding-hello-type" onclick={() => onDone("assistant", hello)}>
            不方便说话？打字问她
          </button>
          <button
            class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110"
            data-testid="onboarding-hello-done"
            onclick={() => onDone("assistant")}
          >
            {helloState === "answered" ? "好的" : "完成"}
          </button>
        </div>
      </div>
    {:else if step === "profile"}
      <div class="relative p-7" data-testid="onboarding-profile">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-primary">第 1 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">选择游戏配置</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              Profile 只决定聊天键、输入方式和术语词库，不会限制你向哪个前台窗口发送。
            </p>
          </div>
        </div>

        {#if loadingResources}
          <p class="mt-5 text-sm text-muted-foreground">正在读取游戏配置…</p>
        {:else if resourceError}
          <div class="mt-5 rounded-lg bg-brand-pink/10 p-3 ring-1 ring-brand-pink/40">
            <p class="text-xs text-brand-pink">{resourceError}</p>
            <button class="mt-2 text-xs text-foreground underline" onclick={() => void reloadResources()}>
              重试
            </button>
          </div>
        {:else}
          <div class="mt-5 grid grid-cols-2 gap-3">
            {#each profiles as profile}
              {@const active = selectedProfileId === profile.id}
              <button
                class="relative rounded-xl p-4 text-left ring-1 transition focus-visible:ring-2 focus-visible:ring-primary {active
                  ? 'bg-primary/12 ring-primary/60'
                  : 'bg-muted/40 ring-hairline hover:bg-muted'}"
                data-testid="profile-{profile.id}"
                onclick={() => void chooseProfile(profile.id)}
              >
                {#if profile.id === "lol"}
                  <span class="absolute top-3 right-3 rounded bg-brand-pink/20 px-2 py-0.5 text-[10px] font-semibold text-brand-pink">
                    推荐
                  </span>
                {/if}
                <span
                  class="flex h-10 w-10 items-center justify-center overflow-hidden rounded-xl {active ? 'bg-primary/15' : 'bg-muted/70'}"
                >
                  {#if profile.icon && iconUrls[profile.id]}
                    <img src={iconUrls[profile.id]} alt={profile.displayName} class="h-full w-full object-contain" />
                  {:else if profile.processNames.length === 0}
                    <!-- lucide: globe -->
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-5 w-5 {active ? 'text-primary' : 'text-muted-foreground'}"><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></svg>
                  {:else}
                    <!-- lucide: gamepad-2 -->
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-5 w-5 {active ? 'text-primary' : 'text-muted-foreground'}"><line x1="6" x2="10" y1="11" y2="11"/><line x1="8" x2="8" y1="9" y2="13"/><line x1="15" x2="15.01" y1="12" y2="12"/><line x1="18" x2="18.01" y1="10" y2="10"/><path d="M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z"/></svg>
                  {/if}
                </span>
                <p class="mt-2 text-sm font-semibold {active ? 'text-primary' : ''}">
                  {profile.displayName}
                </p>
                <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                  {profile.openChatKey} 打开聊天 · {profile.sendKey} 发送 · 热词 {profile.hotwords.length} 个
                </p>
              </button>
            {/each}
          </div>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-muted-foreground hover:text-foreground/90" onclick={() => (step = "welcome")}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
            disabled={loadingResources || !selectedProfile}
            onclick={() => (step = "model")}
          >
            下一步
          </button>
        </div>
      </div>
    {:else if step === "model"}
      <div class="relative p-6" data-testid="onboarding-model">
        <div class="flex items-start gap-4">
          <img src={stickerThinking} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-primary">第 2 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">准备识别模型与麦克风</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              识别完全在本地运行。推荐模型只需下载一次，不会上传你的语音。
            </p>
          </div>
        </div>

        <!-- 先确认存储位置，再下载模型；两项属于同一准备流程。 -->
        <div class="rounded-xl border border-hairline bg-card/70 mt-4 overflow-hidden" data-testid="model-setup-section">
          <div class="border-b border-hairline p-3.5" data-testid="model-storage-section">
            <div class="flex items-center gap-3">
              <span class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-primary/15 text-[11px] font-bold text-primary">
                1
              </span>
              <div class="min-w-0 flex-1">
                <p class="text-sm font-semibold">先确认模型存储位置</p>
                <p
                  class="mt-0.5 truncate text-[11px] text-muted-foreground"
                  title={dirInfo?.dir ?? "正在读取模型存储位置"}
                  data-testid="models-dir-path"
                >
                  {dirInfo?.dir ?? "正在读取…"}{dirInfo?.isDefault ? "（默认）" : ""}
                </p>
              </div>
              <div class="flex shrink-0 items-center gap-1.5">
                {#if dirInfo && !dirInfo.isDefault}
                  <button
                    class="rounded-lg bg-muted/70 px-2.5 py-1.5 text-[11px] text-muted-foreground ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
                    disabled={changingDir || downloadTargetId !== null}
                    onclick={() => void resetModelsDir()}
                  >
                    恢复默认
                  </button>
                {/if}
                <button
                  class="rounded-lg bg-muted/70 px-2.5 py-1.5 text-[11px] font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
                  disabled={changingDir || downloadTargetId !== null || dirInfo === null}
                  onclick={() => void pickModelsDir()}
                >
                  {changingDir ? "切换中…" : "选择位置"}
                </button>
                <button
                  class="rounded-lg bg-muted/70 px-2.5 py-1.5 text-[11px] text-muted-foreground ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
                  disabled={dirInfo === null}
                  onclick={() => void openModelsDir()}
                >
                  打开
                </button>
              </div>
            </div>
            {#if currentModelsDirError}
              <p class="mt-2 text-[11px] text-brand-pink" data-testid="models-dir-error">
                {currentModelsDirError}。请先点击“选择位置”。
              </p>
            {:else}
              <p class="mt-2 text-[11px] text-muted-foreground/80">
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
                  <p class="mt-0.5 text-[11px] text-muted-foreground">
                    约 {Math.round(primaryModel.sizeBytes / 1_000_000)} MB · 中英流式识别
                  </p>
                </div>
                {#if primaryReady}
                  <span class="shrink-0 rounded bg-primary/15 px-3 py-1.5 text-xs font-semibold text-primary">
                    ✓ 已就绪
                  </span>
                {:else}
                  <button
                    class="shrink-0 rounded-lg bg-primary px-3.5 py-1.5 text-xs font-semibold text-primary-foreground disabled:opacity-50"
                    disabled={downloadTargetId !== null || changingDir || dirInfo === null || currentModelsDirError !== null}
                    onclick={() => void downloadById(primaryModel.id)}
                  >
                    {downloadTargetId === primaryModel.id ? "下载中…" : dlError ? "重试下载" : "下载推荐模型"}
                  </button>
                {/if}
              {:else}
                <p class="text-xs text-brand-pink">未找到推荐模型，请重试读取资源。</p>
              {/if}
            </div>
            {#if primaryModel && downloadTargetId === primaryModel.id}
              <div class="mt-2.5 flex items-center gap-2 pl-9">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted/70">
                  <div
                    class="h-full rounded-full bg-primary transition-[width] {dlPercent === null
                      ? 'w-1/3 animate-pulse'
                      : ''}"
                    style:width={dlPercent === null ? undefined : `${dlPercent}%`}
                  ></div>
                </div>
                <span class="w-9 text-right text-[10px] text-muted-foreground">
                  {dlPercent === null ? "连接" : `${dlPercent}%`}
                </span>
                <button
                  class="shrink-0 rounded-lg bg-muted/70 px-2.5 py-1 text-[11px] text-muted-foreground ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
                  disabled={cancellingDownload}
                  onclick={() => void cancelDownloadById()}
                >
                  {cancellingDownload ? "正在取消…" : "取消"}
                </button>
              </div>
            {/if}
            {#if dlError}
              <div class="mt-2 flex items-start gap-2 pl-9">
                <p class="min-w-0 flex-1 text-[11px] text-brand-pink">{dlError}</p>
                {#if primaryModel}
                  <button
                    class="shrink-0 text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground/90 hover:underline"
                    onclick={() => void openManualGuide(primaryModel.id)}
                  >
                    手动下载
                  </button>
                {/if}
              </div>
            {/if}
          </div>
        </div>

        <label class="rounded-xl border border-hairline bg-card/70 mt-3 flex items-center gap-3 p-3">
          <span class="shrink-0 text-sm font-semibold text-primary/90">麦克风</span>
          <div class="min-w-0 flex-1">
            <SelectField
              size="md"
              ariaLabel="麦克风"
              value={selectedDeviceId}
              options={devices.map((device) => ({ value: device.id, label: device.name }))}
              onchange={(id) => void chooseDevice(id)}
            />
          </div>
          <span class="shrink-0 text-[10px] text-muted-foreground/80">下一步会验证</span>
        </label>

        <div class="mt-4 flex items-center justify-between" data-testid="onboarding-model-footer">
          <button class="text-xs text-muted-foreground hover:text-foreground/90" onclick={() => (step = "profile")}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
            disabled={!primaryReady || downloadTargetId !== null}
            onclick={enterHotkeyStep}
          >
            下一步
          </button>
        </div>
      </div>
    {:else if step === "hotkey"}
      <div class="relative p-7" data-testid="onboarding-hotkey">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-primary">第 3 步 / 3</p>
            <h2 class="mt-0.5 text-lg font-bold">设置说话方式与热键</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              选择最顺手的操作方式，点「完成」进入主页后即可开始使用。
            </p>
          </div>
        </div>

        <div class="mt-5 grid grid-cols-3 gap-2">
          {#each modes as mode}
            {@const active = selectedMode === mode.id}
            <button
              class="rounded-xl px-3 py-3 text-left ring-1 transition focus-visible:ring-2 focus-visible:ring-primary {active
                ? 'bg-primary/12 ring-primary/60'
                : 'bg-muted/40 ring-hairline hover:bg-muted'}"
              data-testid="mode-{mode.id}"
              onclick={() => (selectedMode = mode.id)}
            >
              <span
                class="inline-flex h-5 w-5 items-center justify-center {active ? 'text-primary' : 'text-muted-foreground'}"
                aria-hidden="true"
              >
                {#if mode.id === "push-to-talk"}
                  <!-- lucide: mic -->
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>
                {:else if mode.id === "dictation"}
                  <!-- lucide: circle-dot（录音） -->
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3" fill="currentColor" stroke="none"/></svg>
                {:else if mode.id === "one-shot"}
                  <!-- lucide: rocket -->
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>
                {/if}
              </span>
              <p class="mt-1 text-sm font-semibold {active ? 'text-primary' : ''}">{mode.name}</p>
              <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">{mode.desc}</p>
            </button>
          {/each}
        </div>

        {#if checkingInputEnvironment}
          <div
            class="mt-4 rounded-xl bg-primary/8 p-4 ring-1 ring-primary/30"
            data-testid="input-environment-checking"
          >
            <p class="text-sm font-semibold text-primary">正在检测键盘输入环境…</p>
            <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
              正在验证低级键盘钩子与 Windows 模拟输入，无需按下任何按键。
            </p>
          </div>
        {:else if inputEnvironment && !inputEnvironment.available}
          <div
            class="mt-4 rounded-xl bg-brand-pink/10 p-4 ring-1 ring-brand-pink/45"
            data-testid="input-environment-blocked"
          >
            <p class="text-sm font-semibold text-brand-pink">检测到键盘输入被拦截</p>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              Kotone 暂时无法可靠监听热键或发送文字。请将 Kotone 可执行文件加入
              360、火绒等安全软件的信任区，然后重新检测。
            </p>
            {#if inputEnvironment.detail}
              <p class="mt-2 break-all text-[10px] leading-relaxed text-muted-foreground/80">
                检测详情：{inputEnvironment.detail}
              </p>
            {/if}
            <button
              class="mt-3 rounded-lg bg-brand-pink/18 px-3 py-1.5 text-xs font-semibold text-brand-pink ring-1 ring-brand-pink/35 hover:bg-brand-pink/25"
              onclick={() => void runInputEnvironmentCheck()}
            >
              已加入信任区，重新检测
            </button>
            <button
              class="mt-3 rounded-lg bg-muted/70 px-3 py-1.5 text-xs text-muted-foreground ring-1 ring-hairline transition hover:bg-muted"
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
            class="mt-4 rounded-xl bg-primary/8 p-3 ring-1 ring-primary/25"
            data-testid="input-environment-ready"
          >
            <p class="text-xs font-semibold text-primary">✓ 键盘输入环境可用</p>
            {#if !inputEnvironment.hookVerified}
              <p class="mt-1 text-[10px] leading-relaxed text-muted-foreground/80">
                Windows 已接受模拟输入；低级钩子将在你实际录入按键时继续验证。
              </p>
            {/if}
          </div>
        {/if}

        <div class="rounded-xl border border-hairline bg-card/70 mt-4 flex items-center gap-3 p-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs text-muted-foreground">当前热键</p>
            <p class="mt-1 text-lg font-bold text-primary">{currentKey}</p>
          </div>
          <button
            class="rounded-lg bg-muted/70 px-3 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline hover:bg-muted disabled:opacity-60"
            disabled={capturing || checkingInputEnvironment || (inputEnvironment?.available === false && !inputBlockedOverridden)}
            onclick={() => void startCapture()}
          >
            {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "重新录入"}
          </button>
        </div>

        {#if hotkeyWarnings.length > 0}
          <div class="mt-3 rounded-xl bg-brand-pink/10 p-3 ring-1 ring-brand-pink/35">
            <p class="text-xs font-semibold text-brand-pink">键位冲突提示</p>
            {#each hotkeyWarnings as w}
              <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">⚠ {w}</p>
            {/each}
          </div>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-muted-foreground hover:text-foreground/90" onclick={() => (step = "model")}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
            disabled={capturing || checkingInputEnvironment || (inputEnvironment?.available === false && !inputBlockedOverridden)}
            onclick={() => void saveInteractionAndContinue()}
          >
            完成
          </button>
        </div>
      </div>
    {:else if step === "try"}
      <div class="relative p-7" data-testid="onboarding-try">
        <div class="flex items-start gap-4">
          <img src={tried ? stickerCheering : stickerHello} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <h2 class="mt-0.5 text-lg font-bold">说一句试试</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              点一下下面的框，{selectedMode === "push-to-talk" ? "按住" : "按一下"}
              <kbd class="rounded border border-hairline bg-background px-1.5 py-0.5 font-mono text-[11px] text-foreground">{currentKey}</kbd>
              说一句话，比如“打野来下路”。在游戏里，字会这样打进聊天框。
            </p>
          </div>
        </div>
        <textarea
          bind:this={tryBox}
          bind:value={tryText}
          rows="2"
          data-testid="onboarding-try-box"
          aria-label="试一下语音输入"
          class="mt-5 w-full resize-none rounded-xl border border-hairline bg-background px-3 py-2 text-sm outline-none focus:border-primary"
        ></textarea>
        <p class="mt-2 mb-0 min-h-5 text-xs" role="status" data-testid="onboarding-try-status">
          {#if tried}<span class="text-success">✓ 成了，语音输入设好了</span>{:else}<span class="text-muted-foreground">等你开口…</span>{/if}
        </p>
        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-primary hover:underline" data-testid="onboarding-try-ai" onclick={() => { path = "ai"; step = "key"; }}>
            也想让{$companionName}陪你玩？填个 DeepSeek Key
          </button>
          <button
            class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110"
            data-testid="onboarding-try-done"
            onclick={() => onDone()}
          >
            完成
          </button>
        </div>
      </div>
    {/if}

    <div class="relative flex justify-center gap-1.5 pb-4">
      {#each dots as i (i)}
        <span
          class="h-1.5 rounded-full transition-all {i === step
            ? 'w-5 bg-primary'
            : 'w-1.5 bg-muted'}"
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
