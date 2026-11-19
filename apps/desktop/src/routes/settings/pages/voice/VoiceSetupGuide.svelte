<script lang="ts">
  /*
   * 语音输入的引导（ADR 028 W10，V 路径）：原来引导弹窗里“只用语音输入法”的四步，
   * 搬进语音输入子板块顶部，嵌在页面里而不是弹窗：选游戏 → 识别方式 → 热键和交互模式 → 说一句试试。
   * 不需要 Key：没 Key 时只有本地识别可选；在线识别写“需要 DeepSeek Key → 让 Kotone 帮我配置”。
   */
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    cancelDownload, checkInputEnvironment, detectHotkeyConflicts, downloadModel, getModelInstallGuide, getModelsDir,
    getProfileIcon, isDownloadCancelled, isTauri, listModels, listProfiles, modelsDirPathError, setActiveModel,
    setModelsDir, startRuntime, updateSettings,
    type DownloadProgress, type GameProfile, type InputEnvironmentCheck, type InteractionMode, type ModelInfo,
    type ModelInstallGuide, type ModelsDirInfo,
  } from "../../../../lib/ipc";
  import { captureHotkey } from "../../../../lib/hotkeyCapture";
  import { findHotkeyConflict, hotkeyLabel } from "../../../../lib/hotkeyCombo";
  import { appState } from "../../../../lib/stores/state";
  import { runtimeStore } from "../../../../lib/stores/runtime";
  import { errText, settingsStore, toast, toastInfo, toastWarn } from "../../../../lib/stores/ui";
  import ManualDownloadDialog from "../../../../lib/components/ManualDownloadDialog.svelte";

  let { onClose, onConfigure }: {
    onClose: () => void;
    /** 「让 Kotone 帮我配置」：填 DeepSeek Key（在线识别要用）。 */
    onConfigure: () => void;
  } = $props();

  type Step = "profile" | "model" | "hotkey" | "try";
  const STEPS: { id: Step; label: string }[] = [
    { id: "profile", label: "游戏" }, { id: "model", label: "识别" }, { id: "hotkey", label: "热键和模式" }, { id: "try", label: "说一句试试" },
  ];
  let step = $state<Step>("profile");
  const stepIndex = $derived(STEPS.findIndex((item) => item.id === step));

  let loading = $state(true);
  let resourceError = $state("");
  let models = $state<ModelInfo[]>([]);
  let profiles = $state<GameProfile[]>([]);
  let dirInfo = $state<ModelsDirInfo | null>(null);
  let iconUrls = $state<Record<string, string>>({});
  let selectedProfileId = $state("lol");
  let selectedMode = $state<InteractionMode>("push-to-talk");
  let currentKey = $state("CapsLock");

  const primaryModel = $derived(models.find((m) => m.engineId === "sherpa-streaming" && m.backend !== "remote") ?? null);
  const activeModel = $derived(models.find((m) => m.id === ($runtimeStore?.modelId || $settingsStore?.activeModelId)) ?? null);
  const online = $derived($runtimeStore?.asrLocal === false);
  const dirError = $derived(dirInfo ? modelsDirPathError(dirInfo.dir) : null);
  const localReady = $derived(!!primaryModel?.downloaded && dirError === null);
  const recognitionReady = $derived(online || localReady);

  const modes: { id: InteractionMode; name: string; desc: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送" },
    { id: "dictation", name: "录音笔", desc: "按一下开始，再按停止，确认后发送" },
    { id: "one-shot", name: "说一句就走", desc: "按一下，说完自动发送" },
  ];

  onMount(async () => {
    try {
      [models, profiles] = await Promise.all([listModels(), listProfiles()]);
      const settings = $settingsStore;
      selectedProfileId = settings?.activeProfileId && profiles.some((p) => p.id === settings.activeProfileId)
        ? settings.activeProfileId
        : profiles.some((p) => p.id === "lol") ? "lol" : (profiles[0]?.id ?? "generic");
      selectedMode = settings?.interactionMode && settings.interactionMode !== "solo" ? settings.interactionMode : "push-to-talk";
      currentKey = settings?.hotkey.key ?? "CapsLock";
      for (const profile of profiles) void loadIcon(profile);
      dirInfo = await getModelsDir().catch(() => null);
    } catch (error) {
      resourceError = errText(error);
    } finally {
      loading = false;
    }
  });

  async function loadIcon(profile: GameProfile) {
    if (!profile.icon || iconUrls[profile.id] !== undefined) return;
    try {
      const bytes = await getProfileIcon(profile.id);
      if (!bytes.length) { iconUrls[profile.id] = ""; return; }
      let binary = "";
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
      const ext = (profile.icon.split(".").pop() ?? "webp").toLowerCase();
      const mime = ext === "png" ? "image/png" : ext === "jpg" || ext === "jpeg" ? "image/jpeg" : "image/webp";
      iconUrls[profile.id] = `data:${mime};base64,${btoa(binary)}`;
    } catch {
      iconUrls[profile.id] = "";
    }
  }

  async function chooseProfile(id: string) {
    selectedProfileId = id;
    try { settingsStore.set(await updateSettings({ activeProfileId: id })); }
    catch (error) { toast(false, `保存游戏失败：${errText(error)}`); }
  }

  // ---------- 识别：本地模型下载 ----------
  let downloading = $state(false);
  let cancelling = $state(false);
  let progress = $state<{ downloaded: number; total: number } | null>(null);
  let downloadError = $state("");
  let manualGuide = $state<ModelInstallGuide | null>(null);
  let unlistenDownload: (() => void) | undefined;
  let changingDir = $state(false);
  const percent = $derived(progress && progress.total > 0 ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null);

  async function pickModelsDir() {
    if (changingDir || downloading) return;
    let picked: string | null;
    if (isTauri) picked = (await openDialog({ directory: true, title: "选择模型存储位置" }).catch(() => null)) as string | null;
    else picked = window.prompt("模型存储目录（开发预览模式）", dirInfo?.dir ?? "");
    if (!picked) return;
    const pathError = modelsDirPathError(picked);
    if (pathError) { toast(false, pathError); return; }
    changingDir = true;
    try {
      const report = await setModelsDir(picked);
      if (report.failed.length) toastWarn(`目录已切换，但 ${report.failed.length} 项迁移失败（需重新下载）`);
      dirInfo = await getModelsDir();
    } catch (error) {
      toast(false, `切换模型目录失败：${errText(error)}`);
    } finally {
      changingDir = false;
    }
  }

  async function downloadPrimary() {
    const model = primaryModel;
    if (!model || downloading || dirError) return;
    downloading = true;
    progress = null;
    downloadError = "";
    try {
      if (isTauri) {
        unlistenDownload = await listen<DownloadProgress>("kotone://download", (event) => {
          if (event.payload.id === model.id) progress = { downloaded: event.payload.downloaded, total: event.payload.total };
        });
      }
      await downloadModel(model.id);
      models = models.map((m) => (m.id === model.id ? { ...m, downloaded: true } : m));
      // 换成本地识别：选中它，并默认“打开 Kotone 时自动启动”（D9，可在识别卡片里取消）。
      await setActiveModel(model.engineId, model.id);
      settingsStore.set(await updateSettings({ ui: { autoStart: true } }));
      toast(true, "模型下载好了");
    } catch (error) {
      if (isDownloadCancelled(error)) toastInfo("已取消下载，可随时继续");
      else {
        downloadError = errText(error);
        manualGuide = await getModelInstallGuide(model.id).catch(() => null);
      }
    } finally {
      downloading = false;
      cancelling = false;
      unlistenDownload?.();
      unlistenDownload = undefined;
    }
  }

  async function recheckManual() {
    models = await listModels().catch(() => models);
    if (primaryModel?.downloaded) {
      manualGuide = null;
      downloadError = "";
      await setActiveModel(primaryModel.engineId, primaryModel.id).catch(() => {});
      toast(true, "已检测到模型文件");
    } else {
      toastWarn("还没检测到完整模型，请确认文件名和目录后重试");
    }
  }

  // ---------- 热键和模式 ----------
  let capturing = $state(false);
  let captureCleanup: (() => void) | null = null;
  let checkingInput = $state(false);
  let inputEnvironment = $state<InputEnvironmentCheck | null>(null);
  let inputOverridden = $state(false);
  let hotkeyWarnings = $state<string[]>([]);
  const inputBlocked = $derived(inputEnvironment?.available === false && !inputOverridden);

  async function runInputCheck() {
    if (checkingInput) return;
    checkingInput = true;
    try { inputEnvironment = await checkInputEnvironment(); }
    catch (error) { inputEnvironment = { available: false, hookVerified: false, observed: 0, expected: 0, detail: errText(error) }; }
    finally { checkingInput = false; }
  }

  async function recordHotkey() {
    if (capturing || checkingInput || inputBlocked) return;
    capturing = true;
    captureCleanup = await captureHotkey((result) => {
      capturing = false;
      captureCleanup = null;
      if (result.kind === "combo") {
        currentKey = result.combo;
        void detectHotkeyConflicts(result.combo).then((list) => (hotkeyWarnings = list)).catch(() => (hotkeyWarnings = []));
      } else if (result.kind === "error") toast(false, result.message);
    });
  }

  let saving = $state(false);
  async function saveHotkeyAndTry() {
    if (saving) return;
    const conflict = findHotkeyConflict($settingsStore, "record", currentKey);
    if (conflict) { toast(false, `语音输入热键和${conflict}相同，请换一个`); return; }
    saving = true;
    try {
      settingsStore.set(await updateSettings({
        activeProfileId: selectedProfileId,
        interactionMode: selectedMode,
        hotkey: { key: currentKey, mode: selectedMode === "push-to-talk" ? "hold" : "toggle" },
        ui: { firstRunCompleted: true },
      }));
      step = "try";
      void startTry();
    } catch (error) {
      toast(false, `保存失败：${errText(error)}`);
    } finally {
      saving = false;
    }
  }

  // ---------- 说一句试试 ----------
  let tryText = $state("");
  let tryBox = $state<HTMLTextAreaElement | null>(null);
  const tried = $derived(tryText.trim().length > 0 || $appState.state === "success");
  async function startTry() {
    // 本地识别没启动时先启动（在线识别打开即就绪）。
    if ($runtimeStore?.phase !== "running") {
      try { await startRuntime(); } catch (error) { toast(false, `本地语音识别没能启动：${errText(error)}`); }
    }
    tryBox?.focus();
  }

  onDestroy(() => {
    captureCleanup?.();
    unlistenDownload?.();
  });
</script>

<section class="rounded-2xl border border-primary/35 bg-primary/6 p-4" data-testid="voice-guide" aria-label="设好语音输入">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <h2 class="m-0 text-sm font-semibold">四步设好语音输入</h2>
    <button class="text-xs text-muted-foreground hover:text-foreground" data-testid="voice-guide-close" onclick={onClose}>收起</button>
  </div>
  <ol class="mt-3 mb-0 flex list-none flex-wrap gap-1.5 p-0">
    {#each STEPS as item, index (item.id)}
      <li class="rounded-full border px-2.5 py-0.5 text-[11px] {index < stepIndex ? 'border-success/50 text-success' : index === stepIndex ? 'border-primary text-primary' : 'border-hairline text-muted-foreground'}"
        aria-current={index === stepIndex ? "step" : undefined}>{index < stepIndex ? "✓" : `${index + 1}`} {item.label}</li>
    {/each}
  </ol>

  <div class="mt-4 rounded-xl border border-hairline bg-card/80 p-4">
    {#if loading}
      <p class="m-0 text-sm text-muted-foreground">正在读取…</p>
    {:else if resourceError}
      <p class="m-0 text-xs text-destructive">{resourceError}</p>
    {:else if step === "profile"}
      <div data-testid="voice-guide-profile">
        <p class="m-0 text-sm font-semibold">在哪个游戏里用？</p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">决定聊天键、输入方式和游戏术语词库，不限制你往哪个窗口发。</p>
        <div class="mt-3 grid grid-cols-2 gap-2">
          {#each profiles as profile (profile.id)}
            {@const active = selectedProfileId === profile.id}
            <button class="flex items-center gap-2.5 rounded-xl p-3 text-left ring-1 transition {active ? 'bg-primary/12 ring-primary/60' : 'bg-muted/40 ring-hairline hover:bg-muted'}"
              data-testid="profile-{profile.id}" aria-pressed={active} onclick={() => void chooseProfile(profile.id)}>
              <span class="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-muted/70 text-xs">
                {#if iconUrls[profile.id]}<img src={iconUrls[profile.id]} alt="" class="h-full w-full object-contain" />{:else}🎮{/if}
              </span>
              <span class="min-w-0">
                <span class="block truncate text-[13px] font-semibold {active ? 'text-primary' : ''}">{profile.displayName}</span>
                <span class="block truncate text-[11px] text-muted-foreground">{profile.openChatKey} 开聊天 · 热词 {profile.hotwords.length}</span>
              </span>
            </button>
          {/each}
        </div>
      </div>
    {:else if step === "model"}
      <div data-testid="voice-guide-model">
        <p class="m-0 text-sm font-semibold">用什么识别？</p>
        {#if online}
          <p class="mt-1 mb-0 text-xs text-muted-foreground" data-testid="voice-guide-online">
            现在用 {activeModel?.displayName ?? "在线识别"}：打开 Kotone 就能用，不需要启动。
          </p>
        {/if}
        <div class="mt-3 rounded-xl border border-hairline p-3" data-testid="model-setup-section">
          <div class="flex flex-wrap items-center gap-3">
            <div class="min-w-0 flex-1">
              <p class="m-0 text-[13px] font-semibold">{online ? "换成本地识别" : "本地识别"}：免费、离线、边说边出字</p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {primaryModel ? `${primaryModel.displayName.replace(/（.*$/, "")} · 约 ${Math.round(primaryModel.sizeBytes / 1_000_000)} MB` : "推荐模型"} · 下载一次，语音不上传
              </p>
            </div>
            {#if localReady && !online}
              <span class="rounded bg-primary/15 px-2.5 py-1 text-xs font-semibold text-primary">✓ 已就绪</span>
            {:else if primaryModel}
              <button class="rounded-lg bg-primary px-3.5 py-1.5 text-xs font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-download"
                disabled={downloading || changingDir || dirInfo === null || !!dirError}
                onclick={() => void (primaryModel.downloaded ? setActiveModel(primaryModel.engineId, primaryModel.id) : downloadPrimary())}>
                {downloading ? "下载中…" : primaryModel.downloaded ? "换成本地识别" : downloadError ? "重试下载" : online ? "下载并切换" : "下载"}
              </button>
            {/if}
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
            <span class="min-w-0 truncate" data-testid="models-dir-path" title={dirInfo?.dir}>存储位置：{dirInfo?.dir ?? "正在读取…"}</span>
            <button class="text-primary hover:underline disabled:opacity-50" disabled={changingDir || downloading || dirInfo === null} onclick={() => void pickModelsDir()}>{changingDir ? "切换中…" : "更改"}</button>
          </div>
          {#if dirError}<p class="mt-1 mb-0 text-[11px] text-destructive" data-testid="models-dir-error">{dirError}。模型路径需要纯英文，例如 D:\KotoneModels。</p>{/if}
          {#if downloading}
            <div class="mt-2 flex items-center gap-2">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted/70"><div class="h-full rounded-full bg-primary transition-[width] {percent === null ? 'w-1/3 animate-pulse' : ''}" style:width={percent === null ? undefined : `${percent}%`}></div></div>
              <span class="w-9 text-right text-[10px] text-muted-foreground">{percent === null ? "连接" : `${percent}%`}</span>
              <button class="text-[11px] text-muted-foreground hover:text-foreground disabled:opacity-50" disabled={cancelling}
                onclick={() => { cancelling = true; void cancelDownload().catch(() => (cancelling = false)); }}>{cancelling ? "正在取消…" : "取消"}</button>
            </div>
          {/if}
          {#if downloadError}<p class="mt-2 mb-0 text-[11px] text-destructive">{downloadError}</p>{/if}
        </div>
        {#if !online}
          <p class="mt-3 mb-0 text-xs text-muted-foreground" data-testid="voice-guide-online-option">
            在线识别（DeepSeek）：需要 DeepSeek Key →
            <button class="text-primary hover:underline" data-testid="voice-guide-configure" onclick={onConfigure}>让 Kotone 帮我配置</button>
          </p>
        {/if}
      </div>
    {:else if step === "hotkey"}
      <div data-testid="voice-guide-hotkey">
        <p class="m-0 text-sm font-semibold">热键和说话方式</p>
        <div class="mt-3 grid grid-cols-3 gap-2" role="group" aria-label="交互模式">
          {#each modes as mode (mode.id)}
            {@const active = selectedMode === mode.id}
            <button class="rounded-xl px-3 py-2.5 text-left ring-1 transition {active ? 'bg-primary/12 ring-primary/60' : 'bg-muted/40 ring-hairline hover:bg-muted'}"
              data-testid="mode-{mode.id}" aria-pressed={active} onclick={() => (selectedMode = mode.id)}>
              <span class="block text-[13px] font-semibold {active ? 'text-primary' : ''}">{mode.name}</span>
              <span class="mt-0.5 block text-[11px] text-muted-foreground">{mode.desc}</span>
            </button>
          {/each}
        </div>
        <div class="mt-3 flex flex-wrap items-center gap-3 rounded-xl border border-hairline p-3">
          <div class="min-w-0 flex-1">
            <p class="m-0 text-[11px] text-muted-foreground">语音输入热键</p>
            <p class="m-0 text-base font-bold text-primary" data-testid="voice-guide-hotkey-value">{hotkeyLabel(currentKey)}</p>
          </div>
          <button class="rounded-lg border border-hairline px-3 py-1.5 text-xs hover:bg-muted disabled:opacity-50"
            disabled={capturing || checkingInput || inputBlocked} onclick={() => void recordHotkey()}>
            {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "重新录入"}
          </button>
        </div>
        {#each hotkeyWarnings as warning}<p class="mt-1 mb-0 text-[11px] text-warning">⚠ {warning}</p>{/each}
        {#if checkingInput}
          <p class="mt-2 mb-0 text-[11px] text-primary" data-testid="input-environment-checking">正在检测键盘输入环境…</p>
        {:else if inputEnvironment && !inputEnvironment.available}
          <div class="mt-2 rounded-lg bg-brand-pink/10 p-3 ring-1 ring-brand-pink/40" data-testid="input-environment-blocked">
            <p class="m-0 text-xs font-semibold text-brand-pink">检测到键盘输入被拦截</p>
            <p class="mt-1 mb-0 text-[11px] text-muted-foreground">请把 Kotone 加入 360、火绒等安全软件的信任区，然后重新检测。{inputEnvironment.detail ?? ""}</p>
            <div class="mt-2 flex gap-2">
              <button class="rounded-lg bg-brand-pink/18 px-2.5 py-1 text-[11px] text-brand-pink" onclick={() => void runInputCheck()}>重新检测</button>
              <button class="rounded-lg bg-muted/70 px-2.5 py-1 text-[11px] text-muted-foreground" onclick={() => (inputOverridden = true)}>仍然继续</button>
            </div>
          </div>
        {:else if inputEnvironment}
          <p class="mt-2 mb-0 text-[11px] text-success" data-testid="input-environment-ready">✓ 键盘输入环境可用</p>
        {/if}
      </div>
    {:else}
      <div data-testid="voice-guide-try">
        <p class="m-0 text-sm font-semibold">说一句试试</p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">
          点一下下面的框，{selectedMode === "push-to-talk" ? "按住" : "按一下"}
          <kbd class="rounded border border-hairline bg-background px-1.5 py-0.5 font-mono text-[11px]">{hotkeyLabel(currentKey)}</kbd>
          说一句，比如“打野来下路”。在游戏里，字会这样打进聊天框。
        </p>
        <textarea bind:this={tryBox} bind:value={tryText} rows="2" data-testid="onboarding-try-box" aria-label="试一下语音输入"
          class="mt-3 w-full resize-none rounded-xl border border-hairline bg-background px-3 py-2 text-sm outline-none focus:border-primary"></textarea>
        <p class="mt-1 mb-0 min-h-5 text-xs" role="status" data-testid="onboarding-try-status">
          {#if tried}<span class="text-success">✓ 成了，语音输入设好了</span>{:else}<span class="text-muted-foreground">等你开口…</span>{/if}
        </p>
      </div>
    {/if}
  </div>

  <div class="mt-3 flex items-center justify-between">
    <button class="text-xs text-muted-foreground hover:text-foreground disabled:opacity-40" disabled={stepIndex === 0}
      onclick={() => (step = STEPS[Math.max(0, stepIndex - 1)].id)}>← 上一步</button>
    {#if step === "profile"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={loading || !selectedProfileId} onclick={() => (step = "model")}>下一步</button>
    {:else if step === "model"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={!recognitionReady || downloading} onclick={() => { step = "hotkey"; void runInputCheck(); }}>下一步</button>
    {:else if step === "hotkey"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={saving || capturing || checkingInput || inputBlocked} onclick={() => void saveHotkeyAndTry()}>下一步</button>
    {:else}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground" data-testid="voice-guide-done" onclick={onClose}>完成</button>
    {/if}
  </div>
</section>

{#if manualGuide}
  <ManualDownloadDialog guide={manualGuide} error={downloadError} onClose={() => (manualGuide = null)}
    onRetry={() => { manualGuide = null; void downloadPrimary(); }} onRecheck={() => void recheckManual()} />
{/if}
