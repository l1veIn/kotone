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
  import { m } from "$lib/paraglide/messages.js";

  let { onClose, onConfigure }: {
    onClose: () => void;
    /** 「让 Kotone 帮我配置」：填 DeepSeek Key（在线识别要用）。 */
    onConfigure: () => void;
  } = $props();

  type Step = "profile" | "model" | "hotkey" | "try";
  const STEPS: { id: Step; label: string }[] = [
    { id: "profile", label: m.voice_guide_step_profile() }, { id: "model", label: m.voice_guide_step_model() }, { id: "hotkey", label: m.voice_guide_step_hotkey() }, { id: "try", label: m.voice_guide_step_try() },
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
    { id: "push-to-talk", name: m.voice_mode_ptt_name(), desc: m.voice_mode_ptt_desc() },
    { id: "dictation", name: m.voice_mode_dictation_name(), desc: m.voice_guide_mode_dictation_desc() },
    { id: "one-shot", name: m.voice_mode_one_shot_name(), desc: m.voice_guide_mode_one_shot_desc() },
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
    catch (error) { toast(false, m.voice_guide_save_game_failed({ error: errText(error) })); }
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
    if (isTauri) picked = (await openDialog({ directory: true, title: m.voice_guide_pick_dir_title() }).catch(() => null)) as string | null;
    else picked = window.prompt(m.voice_guide_pick_dir_prompt(), dirInfo?.dir ?? "");
    if (!picked) return;
    const pathError = modelsDirPathError(picked);
    if (pathError) { toast(false, pathError); return; }
    changingDir = true;
    try {
      const report = await setModelsDir(picked);
      if (report.failed.length) toastWarn(m.voice_guide_dir_migrate_partial({ count: report.failed.length }));
      dirInfo = await getModelsDir();
    } catch (error) {
      toast(false, m.voice_guide_dir_change_failed({ error: errText(error) }));
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
      toast(true, m.voice_guide_model_downloaded());
    } catch (error) {
      if (isDownloadCancelled(error)) toastInfo(m.voice_guide_download_cancelled());
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
      toast(true, m.voice_guide_model_detected());
    } else {
      toastWarn(m.voice_guide_model_incomplete());
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
    if (conflict) { toast(false, m.voice_guide_hotkey_conflict({ conflict })); return; }
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
      toast(false, m.voice_save_failed({ error: errText(error) }));
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
      try { await startRuntime(); } catch (error) { toast(false, m.voice_guide_runtime_start_failed({ error: errText(error) })); }
    }
    tryBox?.focus();
  }

  onDestroy(() => {
    captureCleanup?.();
    unlistenDownload?.();
  });
</script>

<section class="rounded-2xl border border-primary/35 bg-primary/6 p-4" data-testid="voice-guide" aria-label={m.voice_guide_aria()}>
  <div class="flex flex-wrap items-center justify-between gap-2">
    <h2 class="m-0 text-sm font-semibold">{m.voice_guide_title()}</h2>
    <button class="text-xs text-muted-foreground hover:text-foreground" data-testid="voice-guide-close" onclick={onClose}>{m.voice_guide_close()}</button>
  </div>
  <ol class="mt-3 mb-0 flex list-none flex-wrap gap-1.5 p-0">
    {#each STEPS as item, index (item.id)}
      <li class="rounded-full border px-2.5 py-0.5 text-[11px] {index < stepIndex ? 'border-success/50 text-success' : index === stepIndex ? 'border-primary text-primary' : 'border-hairline text-muted-foreground'}"
        aria-current={index === stepIndex ? "step" : undefined}>{index < stepIndex ? "✓" : `${index + 1}`} {item.label}</li>
    {/each}
  </ol>

  <div class="mt-4 rounded-xl border border-hairline bg-card/80 p-4">
    {#if loading}
      <p class="m-0 text-sm text-muted-foreground">{m.voice_guide_loading()}</p>
    {:else if resourceError}
      <p class="m-0 text-xs text-destructive">{resourceError}</p>
    {:else if step === "profile"}
      <div data-testid="voice-guide-profile">
        <p class="m-0 text-sm font-semibold">{m.voice_guide_profile_title()}</p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.voice_guide_profile_desc()}</p>
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
                <span class="block truncate text-[11px] text-muted-foreground">{m.voice_guide_profile_info({ chatKey: profile.openChatKey, count: profile.hotwords.length })}</span>
              </span>
            </button>
          {/each}
        </div>
      </div>
    {:else if step === "model"}
      <div data-testid="voice-guide-model">
        <p class="m-0 text-sm font-semibold">{m.voice_guide_model_title()}</p>
        {#if online}
          <p class="mt-1 mb-0 text-xs text-muted-foreground" data-testid="voice-guide-online">
            {m.voice_guide_online_now({ name: activeModel?.displayName ?? m.voice_guide_online_default() })}
          </p>
        {/if}
        <div class="mt-3 rounded-xl border border-hairline p-3" data-testid="model-setup-section">
          <div class="flex flex-wrap items-center gap-3">
            <div class="min-w-0 flex-1">
              <p class="m-0 text-[13px] font-semibold">{m.voice_guide_local_title({ name: online ? m.voice_guide_switch_to_local() : m.voice_guide_local_name() })}</p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {m.voice_guide_model_line({ model: primaryModel ? m.voice_guide_model_size({ name: primaryModel.displayName.replace(/（.*$/, ""), size: Math.round(primaryModel.sizeBytes / 1_000_000) }) : m.voice_guide_model_recommended() })}
              </p>
            </div>
            {#if localReady && !online}
              <span class="rounded bg-primary/15 px-2.5 py-1 text-xs font-semibold text-primary">{m.voice_guide_ready()}</span>
            {:else if primaryModel}
              <button class="rounded-lg bg-primary px-3.5 py-1.5 text-xs font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-download"
                disabled={downloading || changingDir || dirInfo === null || !!dirError}
                onclick={() => void (primaryModel.downloaded ? setActiveModel(primaryModel.engineId, primaryModel.id) : downloadPrimary())}>
                {downloading ? m.voice_asr_downloading() : primaryModel.downloaded ? m.voice_guide_switch_to_local() : downloadError ? m.voice_guide_retry_download() : online ? m.voice_asr_download_and_switch() : m.voice_asr_download()}
              </button>
            {/if}
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
            <span class="min-w-0 truncate" data-testid="models-dir-path" title={dirInfo?.dir}>{m.voice_guide_models_dir({ dir: dirInfo?.dir ?? m.voice_guide_loading() })}</span>
            <button class="text-primary hover:underline disabled:opacity-50" disabled={changingDir || downloading || dirInfo === null} onclick={() => void pickModelsDir()}>{changingDir ? m.voice_guide_dir_changing() : m.voice_guide_dir_change()}</button>
          </div>
          {#if dirError}<p class="mt-1 mb-0 text-[11px] text-destructive" data-testid="models-dir-error">{m.voice_guide_dir_error({ error: dirError, example: "D:\\KotoneModels" })}</p>{/if}
          {#if downloading}
            <div class="mt-2 flex items-center gap-2">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted/70"><div class="h-full rounded-full bg-primary transition-[width] {percent === null ? 'w-1/3 animate-pulse' : ''}" style:width={percent === null ? undefined : `${percent}%`}></div></div>
              <span class="w-9 text-right text-[10px] text-muted-foreground">{percent === null ? m.voice_asr_progress_connecting() : `${percent}%`}</span>
              <button class="text-[11px] text-muted-foreground hover:text-foreground disabled:opacity-50" disabled={cancelling}
                onclick={() => { cancelling = true; void cancelDownload().catch(() => (cancelling = false)); }}>{cancelling ? m.voice_asr_cancelling() : m.voice_action_cancel()}</button>
            </div>
          {/if}
          {#if downloadError}<p class="mt-2 mb-0 text-[11px] text-destructive">{downloadError}</p>{/if}
        </div>
        {#if !online}
          <p class="mt-3 mb-0 text-xs text-muted-foreground" data-testid="voice-guide-online-option">
            {m.voice_asr_online_needs_key()}
            <button class="text-primary hover:underline" data-testid="voice-guide-configure" onclick={onConfigure}>{m.voice_shell_configure()}</button>
          </p>
        {/if}
      </div>
    {:else if step === "hotkey"}
      <div data-testid="voice-guide-hotkey">
        <p class="m-0 text-sm font-semibold">{m.voice_guide_hotkey_title()}</p>
        <div class="mt-3 grid grid-cols-3 gap-2" role="group" aria-label={m.voice_general_mode_title()}>
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
            <p class="m-0 text-[11px] text-muted-foreground">{m.voice_hotkey_name_record()}</p>
            <p class="m-0 text-base font-bold text-primary" data-testid="voice-guide-hotkey-value">{hotkeyLabel(currentKey)}</p>
          </div>
          <button class="rounded-lg border border-hairline px-3 py-1.5 text-xs hover:bg-muted disabled:opacity-50"
            disabled={capturing || checkingInput || inputBlocked} onclick={() => void recordHotkey()}>
            {capturing ? m.voice_hotkey_capturing() : m.voice_guide_rerecord()}
          </button>
        </div>
        {#each hotkeyWarnings as warning}<p class="mt-1 mb-0 text-[11px] text-warning">⚠ {warning}</p>{/each}
        {#if checkingInput}
          <p class="mt-2 mb-0 text-[11px] text-primary" data-testid="input-environment-checking">{m.voice_guide_input_checking()}</p>
        {:else if inputEnvironment && !inputEnvironment.available}
          <div class="mt-2 rounded-lg bg-brand-pink/10 p-3 ring-1 ring-brand-pink/40" data-testid="input-environment-blocked">
            <p class="m-0 text-xs font-semibold text-brand-pink">{m.voice_guide_input_blocked_title()}</p>
            <p class="mt-1 mb-0 text-[11px] text-muted-foreground">{m.voice_guide_input_blocked_desc({ detail: inputEnvironment.detail ?? "" })}</p>
            <div class="mt-2 flex gap-2">
              <button class="rounded-lg bg-brand-pink/18 px-2.5 py-1 text-[11px] text-brand-pink" onclick={() => void runInputCheck()}>{m.voice_guide_input_recheck()}</button>
              <button class="rounded-lg bg-muted/70 px-2.5 py-1 text-[11px] text-muted-foreground" onclick={() => (inputOverridden = true)}>{m.voice_guide_input_continue()}</button>
            </div>
          </div>
        {:else if inputEnvironment}
          <p class="mt-2 mb-0 text-[11px] text-success" data-testid="input-environment-ready">{m.voice_guide_input_ready()}</p>
        {/if}
      </div>
    {:else}
      <div data-testid="voice-guide-try">
        <p class="m-0 text-sm font-semibold">{m.voice_guide_step_try()}</p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">
          {m.voice_guide_try_before({ action: selectedMode === "push-to-talk" ? m.voice_guide_try_hold() : m.voice_guide_try_press() })}
          <kbd class="rounded border border-hairline bg-background px-1.5 py-0.5 font-mono text-[11px]">{hotkeyLabel(currentKey)}</kbd>
          {m.voice_guide_try_after()}
        </p>
        <textarea bind:this={tryBox} bind:value={tryText} rows="2" data-testid="onboarding-try-box" aria-label={m.voice_guide_try_box_label()}
          class="mt-3 w-full resize-none rounded-xl border border-hairline bg-background px-3 py-2 text-sm outline-none focus:border-primary"></textarea>
        <p class="mt-1 mb-0 min-h-5 text-xs" role="status" data-testid="onboarding-try-status">
          {#if tried}<span class="text-success">{m.voice_guide_try_done()}</span>{:else}<span class="text-muted-foreground">{m.voice_guide_try_waiting()}</span>{/if}
        </p>
      </div>
    {/if}
  </div>

  <div class="mt-3 flex items-center justify-between">
    <button class="text-xs text-muted-foreground hover:text-foreground disabled:opacity-40" disabled={stepIndex === 0}
      onclick={() => (step = STEPS[Math.max(0, stepIndex - 1)].id)}>{m.voice_guide_prev()}</button>
    {#if step === "profile"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={loading || !selectedProfileId} onclick={() => (step = "model")}>{m.voice_guide_next()}</button>
    {:else if step === "model"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={!recognitionReady || downloading} onclick={() => { step = "hotkey"; void runInputCheck(); }}>{m.voice_guide_next()}</button>
    {:else if step === "hotkey"}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground disabled:opacity-50" data-testid="voice-guide-next"
        disabled={saving || capturing || checkingInput || inputBlocked} onclick={() => void saveHotkeyAndTry()}>{m.voice_guide_next()}</button>
    {:else}
      <button class="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground" data-testid="voice-guide-done" onclick={onClose}>{m.voice_action_done()}</button>
    {/if}
  </div>
</section>

{#if manualGuide}
  <ManualDownloadDialog guide={manualGuide} error={downloadError} onClose={() => (manualGuide = null)}
    onRetry={() => { manualGuide = null; void downloadPrimary(); }} onRecheck={() => void recheckManual()} />
{/if}
