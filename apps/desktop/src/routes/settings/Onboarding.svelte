<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    downloadModel,
    getHotkeyStatus,
    isTauri,
    listAudioDevices,
    listModels,
    listProfiles,
    setAudioDevice,
    simulateSend,
    startRuntime,
    updateSettings,
    type AudioDevice,
    type DownloadProgress,
    type GameProfile,
    type InteractionMode,
    type ModelInfo,
  } from "../../lib/ipc";
  import { captureHotkey } from "../../lib/hotkeyCapture";
  import { appState } from "../../lib/stores/state";
  import { runtimeStore } from "../../lib/stores/runtime";
  import {
    errText,
    settingsStore,
    toast,
    toastInfo,
    toastWarn,
  } from "../../lib/stores/ui";
  import relayRoomBg from "../../assets/brand/relay-room-bg.png";
  import kotoneCutout from "../../assets/brand/kotone-cutout.png";
  import stickerHello from "../../assets/brand/stickers/hello.png";
  import stickerThinking from "../../assets/brand/stickers/thinking.png";
  import stickerCheering from "../../assets/brand/stickers/cheering.png";
  import stickerProud from "../../assets/brand/stickers/proud.png";

  let { onDone }: { onDone: () => void } = $props();

  const LAST_STEP = 4;
  let step = $state(0);
  let loadingResources = $state(true);
  let resourceError = $state("");
  let models = $state<ModelInfo[]>([]);
  let profiles = $state<GameProfile[]>([]);
  let devices = $state<AudioDevice[]>([]);
  let downloadedIds = $state<string[]>([]);

  const primaryModel = $derived(
    models.find((m) => m.engineId === "sherpa-onnx-x-asr-zh-en") ?? null,
  );
  const isDownloaded = (m: ModelInfo | null) =>
    m !== null && (m.downloaded || downloadedIds.includes(m.id));
  const primaryReady = $derived(isDownloaded(primaryModel));

  let selectedProfileId = $state("lol");
  let selectedDeviceId = $state("default");
  let selectedMode = $state<InteractionMode>("push-to-talk");
  let currentKey = $state("F8");

  const modes: { id: InteractionMode; name: string; desc: string; icon: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送", icon: "🎙️" },
    { id: "dictation", name: "录音笔", desc: "按一下开始，再按停止，确认后发送", icon: "⏺️" },
    { id: "one-shot", name: "说一句就走", desc: "按一下，说完自动发送", icon: "🚀" },
  ];

  const selectedProfile = $derived(
    profiles.find((p) => p.id === selectedProfileId) ?? null,
  );
  const modeInstruction = $derived(
    selectedMode === "push-to-talk"
      ? `按住 ${currentKey} 说话，松开后会直接发送`
      : selectedMode === "dictation"
        ? `按 ${currentKey} 开始，再按一次停止；预览出现后再按一次发送`
        : `按 ${currentKey} 开始，说完停顿片刻后自动发送`,
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
      currentKey = settings?.hotkey.key ?? "F8";
    } catch (e) {
      resourceError = errText(e);
    } finally {
      loadingResources = false;
    }
  });

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

  const dlPercent = $derived(
    dlProgress && dlProgress.total > 0
      ? Math.min(100, Math.round((dlProgress.downloaded / dlProgress.total) * 100))
      : null,
  );

  async function downloadById(id: string) {
    if (downloadTargetId) return;
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
      dlError = errText(e);
      toast(false, `模型下载失败：${dlError}`);
    } finally {
      downloadTargetId = null;
      unlistenDl?.();
      unlistenDl = undefined;
    }
  }

  let capturing = $state(false);
  let captureCleanup: (() => void) | null = null;

  async function startCapture() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((result) => {
      capturing = false;
      captureCleanup = null;
      if (result.kind === "combo") {
        currentKey = result.combo;
        toast(true, `已录入热键：${result.combo}`);
      } else if (result.kind === "cancelled") {
        toastInfo("已取消录入");
      } else if (result.kind === "timeout") {
        toastWarn("录入超时，请重试");
      } else {
        toast(false, result.message);
      }
    });
  }

  async function saveInteractionAndContinue() {
    try {
      const mode = selectedMode === "push-to-talk" ? "hold" : "toggle";
      settingsStore.set(
        await updateSettings({
          interactionMode: selectedMode,
          hotkey: { key: currentKey, mode },
        }),
      );
      step = LAST_STEP;
    } catch (e) {
      toast(false, `保存说话方式失败：${errText(e)}`);
    }
  }

  let starting = $state(false);
  let runtimeReady = $state(false);
  let hotkeyReady = $state(false);
  let testError = $state("");
  let testArmed = $state(false);
  let testInput = $state("");
  let testReceived = $state("");
  let testSucceeded = $state(false);
  let trainingInput: HTMLInputElement | undefined = $state();

  async function startForTest() {
    if (starting) return;
    if (!primaryReady) {
      testError = "请先下载推荐识别模型";
      return;
    }
    starting = true;
    testError = "";
    try {
      // VAD 判停组件已随应用本体分发，one-shot 无需额外下载
      const hotkeyMode = selectedMode === "push-to-talk" ? "hold" : "toggle";
      settingsStore.set(
        await updateSettings({
          activeProfileId: selectedProfileId,
          interactionMode: selectedMode,
          hotkey: { key: currentKey, mode: hotkeyMode },
        }),
      );
      runtimeStore.set(await startRuntime());
      const status = await getHotkeyStatus();
      hotkeyReady = status.registered;
      if (!status.registered) {
        throw new Error(status.error ?? "热键未成功注册");
      }
      runtimeReady = true;
      await tick();
      trainingInput?.focus();
    } catch (e) {
      runtimeReady = false;
      hotkeyReady = false;
      testError = errText(e);
      toast(false, `启动失败：${testError}`);
    } finally {
      starting = false;
    }
  }

  function acceptTrainingMessage() {
    const text = testInput.trim();
    if (!text) return;
    testReceived = text;
    testSucceeded = true;
    testArmed = false;
    toast(true, "完整发送测试通过 ✨");
  }

  function handleTrainingKeydown(event: KeyboardEvent) {
    if (event.key !== "Enter" || !runtimeReady) return;
    event.preventDefault();
    if (!testArmed) {
      testArmed = true;
      testInput = "";
      testReceived = "";
      testSucceeded = false;
      return;
    }
    acceptTrainingMessage();
  }

  async function runTextInjectionTest() {
    if (!runtimeReady) return;
    testArmed = false;
    testInput = "";
    testReceived = "";
    testSucceeded = false;
    testError = "";
    await tick();
    trainingInput?.focus();
    try {
      await new Promise<void>((resolve) => setTimeout(resolve, 80));
      if (isTauri) {
        await simulateSend("琴音测试发送", selectedProfileId);
      } else {
        // dev:web 的可重复 E2E：生产环境不会显示或执行这条模拟分支。
        testArmed = true;
        testInput = "琴音测试发送";
        acceptTrainingMessage();
      }
    } catch (e) {
      testError = errText(e);
      toast(false, `文字注入测试失败：${testError}`);
    }
  }

  $effect(() => {
    if ($appState.state === "error" && $appState.errorMessage) {
      testError = $appState.errorMessage;
    }
  });

  let finishing = $state(false);
  async function finish(skipped = false) {
    if (finishing || (!testSucceeded && !skipped)) return;
    finishing = true;
    try {
      settingsStore.set(
        await updateSettings({
          activeProfileId: selectedProfileId,
          ui: { firstRunCompleted: true },
        }),
      );
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
        <div
          class="pointer-events-none absolute inset-0 opacity-25"
          style:background-image="url({relayRoomBg})"
          style:background-size="cover"
          style:background-position="center"
        ></div>
        <div class="relative flex items-center gap-6 p-8">
          <div class="min-w-0 flex-1">
            <img src={stickerHello} alt="" class="mb-3 h-12 w-12" />
            <h1 class="text-2xl font-bold">
              欢迎来到 <span class="kotone-gradient-text">Kotone 琴音</span>
            </h1>
            <p class="mt-2 text-sm leading-relaxed text-white/65">
              游戏里不动手，说话就能发消息。<br />
              接下来会选游戏配置、准备本地模型、设置热键，并完成一次真实发送测试。
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
          <img
            src={kotoneCutout}
            alt="Kotone 看板娘"
            class="h-44 w-auto shrink-0 drop-shadow-[0_0_24px_rgba(0,229,255,0.35)]"
          />
        </div>
      </div>
    {:else if step === 1}
      <div class="relative p-7" data-testid="onboarding-profile">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 1 步 / 4</p>
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
      <div class="relative p-7" data-testid="onboarding-model">
        <div class="flex items-start gap-4">
          <img src={stickerThinking} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 2 步 / 4</p>
            <h2 class="mt-0.5 text-lg font-bold">准备识别模型与麦克风</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              识别完全在本地运行。推荐模型只需下载一次，不会上传你的语音。
            </p>
          </div>
        </div>

        <div class="kotone-card mt-5 p-4">
          {#if primaryModel}
            <div class="flex items-center justify-between gap-4">
              <div class="min-w-0">
                <p class="text-sm font-semibold">{primaryModel.displayName}</p>
                <p class="mt-1 text-[11px] text-white/45">
                  约 {Math.round(primaryModel.sizeBytes / 1_000_000)} MB · 中英流式识别
                </p>
              </div>
              {#if primaryReady}
                <span class="shrink-0 rounded bg-kotone-cyan/15 px-3 py-1.5 text-xs font-semibold text-kotone-cyan">
                  ✓ 已就绪
                </span>
              {:else}
                <button
                  class="shrink-0 rounded-lg bg-kotone-cyan px-3.5 py-1.5 text-xs font-semibold text-kotone-deep disabled:opacity-60"
                  disabled={downloadTargetId !== null}
                  onclick={() => primaryModel && void downloadById(primaryModel.id)}
                >
                  {downloadTargetId === primaryModel.id ? "下载中…" : dlError ? "重试下载" : "下载推荐模型"}
                </button>
              {/if}
            </div>
            {#if downloadTargetId === primaryModel.id}
              <div class="mt-3 h-1.5 overflow-hidden rounded-full bg-white/10">
                <div
                  class="h-full rounded-full bg-kotone-cyan transition-[width] {dlPercent === null
                    ? 'w-1/3 animate-pulse'
                    : ''}"
                  style:width={dlPercent === null ? undefined : `${dlPercent}%`}
                ></div>
              </div>
              <p class="mt-1.5 text-[11px] text-white/45">
                {dlPercent === null ? "建立连接中…" : `${dlPercent}%`}
              </p>
            {/if}
          {:else}
            <p class="text-xs text-kotone-pink">未找到推荐模型，请重试读取资源。</p>
          {/if}
        </div>

        <label class="kotone-card mt-3 block p-4">
          <span class="text-sm font-semibold text-kotone-cyan/90">麦克风</span>
          <select
            class="mt-3 w-full rounded-lg bg-white/8 px-2.5 py-2 text-sm ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
            value={selectedDeviceId}
            onchange={(e) => void chooseDevice((e.target as HTMLSelectElement).value)}
          >
            {#each devices as device}
              <option value={device.id}>{device.name}</option>
            {/each}
          </select>
          <span class="mt-2 block text-[11px] text-white/45">
            最后一步启动后会通过真实录音验证这个设备。
          </span>
        </label>

        {#if dlError}
          <p class="mt-3 text-xs text-kotone-pink">{dlError}</p>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 1)}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-50"
            disabled={!primaryReady || downloadTargetId !== null}
            onclick={() => (step = 3)}
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
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 3 步 / 4</p>
            <h2 class="mt-0.5 text-lg font-bold">设置说话方式与热键</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              选择最顺手的操作方式。最后一步会让你现场按一次，确认游戏中真的可用。
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

        <div class="kotone-card mt-4 flex items-center gap-3 p-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs text-white/50">当前热键</p>
            <p class="mt-1 text-lg font-bold text-kotone-cyan">{currentKey}</p>
          </div>
          <button
            class="rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/85 ring-1 ring-white/15 hover:bg-white/20 disabled:opacity-60"
            disabled={capturing}
            onclick={() => void startCapture()}
          >
            {capturing ? "请按下组合键…（Esc 取消）" : "重新录入"}
          </button>
        </div>

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 2)}>
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-50"
            disabled={capturing}
            onclick={() => void saveInteractionAndContinue()}
          >
            去测试
          </button>
        </div>
      </div>
    {:else}
      <div class="relative p-7" data-testid="onboarding-test">
        <div class="flex items-start gap-4">
          <img src={testSucceeded ? stickerProud : stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div class="min-w-0 flex-1">
            <p class="text-[11px] font-semibold tracking-wide text-kotone-cyan">第 4 步 / 4</p>
            <h2 class="mt-0.5 text-lg font-bold">
              {testSucceeded ? "测试通过，可以开玩！" : "启动并发送第一条消息"}
            </h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              这一步会验证模型、麦克风、全局热键、{selectedProfile?.displayName ?? "当前"} Profile
              和文字注入的完整链路。
            </p>
          </div>
        </div>

        <div class="mt-4 grid grid-cols-3 gap-2 text-[11px]">
          <div class="rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/10">
            <p class="text-white/45">Profile</p>
            <p class="mt-0.5 truncate font-semibold">{selectedProfile?.displayName ?? selectedProfileId}</p>
          </div>
          <div class="rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/10">
            <p class="text-white/45">识别模型</p>
            <p class="mt-0.5 font-semibold text-kotone-cyan">✓ 已就绪</p>
          </div>
          <div class="rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/10">
            <p class="text-white/45">全局热键</p>
            <p class="mt-0.5 font-semibold {hotkeyReady ? 'text-kotone-cyan' : ''}">
              {hotkeyReady ? `✓ ${currentKey} 已注册` : currentKey}
            </p>
          </div>
        </div>

        {#if !runtimeReady}
          <div class="mt-4 rounded-xl bg-kotone-card/60 p-5 text-center ring-1 ring-white/10">
            <p class="text-sm font-semibold">先启动琴音，加载模型并注册全局热键</p>
            <button
              class="mt-4 rounded-lg bg-kotone-pink px-5 py-2 text-sm font-bold text-white shadow-glow-pink transition hover:brightness-110 disabled:opacity-60"
              disabled={starting || downloadTargetId !== null}
              onclick={() => void startForTest()}
            >
              {starting ? "启动中…" : "▶ 启动琴音"}
            </button>
          </div>
        {:else}
          <div class="mt-4 rounded-xl bg-kotone-card/60 p-4 ring-1 ring-kotone-cyan/35">
            <div class="flex items-start justify-between gap-4">
              <div>
                <p class="text-sm font-semibold text-kotone-cyan">训练聊天框已就绪</p>
                <p class="mt-1 text-[11px] leading-relaxed text-white/55">{modeInstruction}</p>
              </div>
              <span class="shrink-0 rounded bg-kotone-cyan/15 px-2 py-1 text-[10px] font-semibold text-kotone-cyan">
                运行中
              </span>
            </div>

            <label class="mt-3 block">
              <span class="sr-only">训练聊天输入框</span>
              <input
                bind:this={trainingInput}
                bind:value={testInput}
                data-testid="training-input"
                class="w-full rounded-lg bg-kotone-deep/80 px-3 py-3 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/35 focus:ring-2 focus:ring-kotone-cyan"
                placeholder={testArmed ? "正在接收琴音发送的文字…" : "点击这里保持焦点，然后按热键说话"}
                autocomplete="off"
                spellcheck="false"
                onkeydown={handleTrainingKeydown}
              />
            </label>
            <div class="mt-3 flex items-center justify-between gap-3">
              <p class="text-[11px] text-white/45">
                识别状态：{$appState.state === "idle" ? "等待热键" : $appState.state}
                {#if $appState.partialText}
                  · {$appState.partialText}
                {/if}
              </p>
              <button
                class="shrink-0 text-[11px] text-white/55 underline underline-offset-2 hover:text-white"
                onclick={() => void runTextInjectionTest()}
              >
                只测试文字发送
              </button>
            </div>
          </div>
        {/if}

        {#if testSucceeded}
          <div
            class="mt-4 rounded-xl bg-kotone-cyan/10 p-4 ring-1 ring-kotone-cyan/50"
            data-testid="test-success"
            role="status"
          >
            <p class="text-sm font-semibold text-kotone-cyan">✓ 完整发送测试通过</p>
            <p class="mt-1 break-all text-sm text-white">“{testReceived}”</p>
          </div>
        {:else if testError}
          <div class="mt-4 rounded-xl bg-kotone-pink/10 p-3 ring-1 ring-kotone-pink/45">
            <p class="text-xs font-semibold text-kotone-pink">测试没有完成</p>
            <p class="mt-1 break-all text-[11px] text-white/60">{testError}</p>
            <p class="mt-1 text-[11px] text-white/45">修复后可以留在这里直接重试。</p>
          </div>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button class="text-xs text-white/50 hover:text-white/80" onclick={() => (step = 3)}>
            ← 上一步
          </button>
          <div class="flex items-center gap-3">
            {#if !testSucceeded}
              <button
                class="text-xs text-white/50 underline-offset-2 hover:text-white/80 hover:underline"
                onclick={() => void finish(true)}
              >
                跳过测试并完成
              </button>
            {/if}
            <button
              class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 disabled:opacity-40"
              data-testid="finish-onboarding"
              disabled={!testSucceeded || finishing}
              onclick={() => void finish(false)}
            >
              {finishing ? "保存中…" : "测试成功，开始使用"}
            </button>
          </div>
        </div>
      </div>
    {/if}

    <div class="relative flex justify-center gap-1.5 pb-4">
      {#each [0, 1, 2, 3, 4] as i}
        <span
          class="h-1.5 rounded-full transition-all {i === step
            ? 'w-5 bg-kotone-cyan shadow-glow-cyan'
            : 'w-1.5 bg-white/20'}"
        ></span>
      {/each}
    </div>
  </div>
</div>
