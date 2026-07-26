<script lang="ts">
  /*
   * 首启向导（Settings.ui.firstRunCompleted === false 时由 Settings.svelte 弹层）。
   * 三步：欢迎 → 推荐模型下载 → 交互模式与热键。完成或任一处「跳过」都会
   * 落盘 ui.firstRunCompleted = true（update_settings 深合并，无新 IPC）。
   * 热键录入复用 lib/hotkeyCapture.ts（与快捷键页同一 helper）。
   */
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    downloadModel,
    listModels,
    updateSettings,
    type DownloadProgress,
    type InteractionMode,
    type ModelInfo,
  } from "../../lib/ipc";
  import { captureHotkey } from "../../lib/hotkeyCapture";
  import { settingsStore, toast, toastInfo, toastWarn, errText } from "../../lib/stores/ui";
  import relayRoomBg from "../../assets/brand/relay-room-bg.png";
  import kotoneCutout from "../../assets/brand/kotone-cutout.png";
  import stickerHello from "../../assets/brand/stickers/hello.png";
  import stickerThinking from "../../assets/brand/stickers/thinking.png";
  import stickerCheering from "../../assets/brand/stickers/cheering.png";

  let { onDone }: { onDone: () => void } = $props();

  let step = $state(0);

  // ---------- 第 2 步：推荐模型下载 ----------
  let models = $state<ModelInfo[]>([]);
  /** 推荐卡：优先 sherpa 未下载模型，其次任意未下载模型 */
  const recommended = $derived(
    models.find((m) => m.engineId.startsWith("sherpa") && !m.downloaded) ??
      models.find((m) => !m.downloaded) ??
      null,
  );
  let downloading = $state(false);
  let dlProgress = $state<{ downloaded: number; total: number } | null>(null);
  let dlDone = $state(false);
  let dlError = $state<string | null>(null);
  let unlistenDl: (() => void) | undefined;

  const dlPercent = $derived(
    dlProgress && dlProgress.total > 0
      ? Math.min(100, Math.round((dlProgress.downloaded / dlProgress.total) * 100))
      : null,
  );

  onMount(async () => {
    try {
      models = await listModels();
    } catch {
      /* 清单拉取失败不阻断向导：推荐卡为空，直接可跳过 */
    }
  });

  async function startDownload() {
    if (!recommended || downloading) return;
    downloading = true;
    dlError = null;
    dlProgress = null;
    const id = recommended.id;
    unlistenDl = await listen<DownloadProgress>("kotone://download", (ev) => {
      if (ev.payload.id === id) {
        dlProgress = { downloaded: ev.payload.downloaded, total: ev.payload.total };
      }
    });
    try {
      await downloadModel(id);
      dlDone = true;
      toast(true, `模型下载完成：${id}`);
    } catch (e) {
      dlError = errText(e);
      toast(false, `模型下载失败：${dlError}`);
    } finally {
      downloading = false;
      unlistenDl?.();
      unlistenDl = undefined;
    }
  }

  // ---------- 第 3 步：交互模式 + 热键 ----------
  const modes: { id: InteractionMode; name: string; desc: string; icon: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送", icon: "🎙️" },
    { id: "dictation", name: "录音笔", desc: "点按开始，再点停止", icon: "⏺️" },
    { id: "one-shot", name: "说一句就走", desc: "说完自动停、自动发", icon: "🚀" },
  ];
  const currentMode = $derived($settingsStore?.interactionMode ?? null);
  const currentKey = $derived($settingsStore?.hotkey.key ?? "F8");
  let capturing = $state(false);
  let captureCleanup: (() => void) | null = null;

  async function selectMode(id: InteractionMode) {
    try {
      // 预设与 hotkey.mode 同步落盘（与 HotkeyPage 同一约定）：
      // 对讲机 = 按住，录音笔 / 说一句就走 = 点按
      settingsStore.set(
        await updateSettings({
          interactionMode: id,
          hotkey: { key: currentKey, mode: id === "push-to-talk" ? "hold" : "toggle" },
        }),
      );
      const m = modes.find((x) => x.id === id);
      toast(true, `交互模式已切换：${m?.name ?? id}`);
    } catch (e) {
      toast(false, `切换模式失败：${errText(e)}`);
    }
  }

  async function startCapture() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((r) => {
      capturing = false;
      captureCleanup = null;
      if (r.kind === "combo") {
        void saveKey(r.combo);
      } else if (r.kind === "cancelled") {
        toastInfo("已取消录入");
      } else if (r.kind === "timeout") {
        toastWarn("录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  async function saveKey(key: string) {
    try {
      settingsStore.set(
        await updateSettings({
          hotkey: { key, mode: $settingsStore?.hotkey.mode ?? "toggle" },
        }),
      );
      toast(true, `热键已保存并生效：${key}`);
    } catch (e) {
      toast(false, `保存热键失败：${errText(e)}`);
    }
  }

  onDestroy(() => {
    captureCleanup?.();
    unlistenDl?.();
  });

  // ---------- 完成 / 跳过（都会落盘，下次不再弹） ----------
  let finishing = $state(false);
  async function finish() {
    if (finishing) return;
    finishing = true;
    try {
      settingsStore.set(await updateSettings({ ui: { firstRunCompleted: true } }));
    } catch {
      /* 落盘失败不阻断关闭：下次启动会再弹一次 */
    }
    onDone();
  }
</script>

<div class="absolute inset-0 z-50 flex items-center justify-center bg-kotone-deep/92 p-6 backdrop-blur-sm">
  <div class="kotone-panel relative w-full max-w-xl overflow-hidden shadow-glow-cyan-lg">
    <!-- 顶部光晕 -->
    <div
      class="pointer-events-none absolute -top-24 left-1/2 h-48 w-96 -translate-x-1/2 rounded-full bg-kotone-violet/25 blur-3xl"
    ></div>

    {#if step === 0}
      <!-- 第 1 步：欢迎 -->
      <div class="relative">
        <div
          class="pointer-events-none absolute inset-0 opacity-25"
          style:background-image="url({relayRoomBg})"
          style:background-size="cover"
          style:background-position="center"
        ></div>
        <div class="relative flex items-center gap-5 p-8">
          <div class="min-w-0 flex-1">
            <img src={stickerHello} alt="" class="mb-3 h-12 w-12" />
            <h1 class="text-2xl font-bold">
              欢迎来到 <span class="kotone-gradient-text">Kotone 琴音</span>
            </h1>
            <p class="mt-2 text-sm leading-relaxed text-white/65">
              游戏里不动手，说话就能发消息。<br />
              接下来两步：下载识别模型、设定你的说话方式，一分钟搞定。
            </p>
            <div class="mt-6 flex items-center gap-3">
              <button
                class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95"
                onclick={() => (step = 1)}
              >
                开始使用
              </button>
              <button
                class="text-xs text-white/45 underline-offset-2 transition hover:text-white/75 hover:underline"
                onclick={() => void finish()}
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
      <!-- 第 2 步：推荐模型下载 -->
      <div class="relative p-8">
        <div class="flex items-start gap-4">
          <img src={stickerThinking} alt="" class="h-12 w-12 shrink-0" />
          <div>
            <h2 class="text-lg font-bold">下载识别模型</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              语音识别完全在本地运行，模型只需下载一次。推荐 sherpa-onnx 中文流式模型，边识别边出字。
            </p>
          </div>
        </div>

        {#if recommended}
          <div class="kotone-card mt-5 p-4 ring-1 ring-white/10">
            <div class="flex items-center justify-between gap-3">
              <div class="min-w-0">
                <p class="truncate text-sm font-semibold">{recommended.id}</p>
                <p class="mt-0.5 text-[11px] text-white/45">
                  {recommended.engineId} · 约 {Math.round(recommended.sizeBytes / 1_000_000)} MB
                </p>
              </div>
              {#if dlDone}
                <span class="shrink-0 rounded bg-kotone-cyan/15 px-2.5 py-1 text-xs font-semibold text-kotone-cyan">
                  ✓ 已就绪
                </span>
              {:else}
                <button
                  class="shrink-0 rounded-lg bg-kotone-cyan px-3.5 py-1.5 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-60"
                  disabled={downloading}
                  onclick={() => void startDownload()}
                >
                  {downloading ? "下载中…" : dlError ? "重试下载" : "下载"}
                </button>
              {/if}
            </div>
            {#if downloading}
              <div class="mt-3 h-1.5 overflow-hidden rounded-full bg-white/10">
                {#if dlPercent !== null}
                  <div
                    class="h-full rounded-full bg-kotone-cyan shadow-glow-cyan transition-[width]"
                    style:width="{dlPercent}%"
                  ></div>
                {:else}
                  <div class="h-full w-1/3 animate-pulse rounded-full bg-kotone-cyan/70"></div>
                {/if}
              </div>
              <p class="mt-1.5 text-[11px] text-white/45">
                {dlPercent !== null ? `${dlPercent}%` : "建立连接中…"}
              </p>
            {/if}
          </div>
        {:else}
          <p class="mt-5 rounded-lg bg-white/5 p-3 text-xs text-white/55 ring-1 ring-white/10">
            没有待下载的推荐模型（可能已全部就绪），稍后可到「引擎与模型」页管理。
          </p>
        {/if}

        <div class="mt-6 flex items-center justify-between">
          <button
            class="text-xs text-white/45 underline-offset-2 transition hover:text-white/75 hover:underline"
            onclick={() => (step = 2)}
          >
            跳过，稍后再下
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
            disabled={downloading}
            onclick={() => (step = 2)}
          >
            下一步
          </button>
        </div>
      </div>
    {:else}
      <!-- 第 3 步：交互模式 + 热键 -->
      <div class="relative p-8">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-12 w-12 shrink-0" />
          <div>
            <h2 class="text-lg font-bold">设定你的说话方式</h2>
            <p class="mt-1 text-xs leading-relaxed text-white/55">
              选一种交互模式，再录一个顺手的热键。之后随时可在「快捷键」页调整。
            </p>
          </div>
        </div>

        <div class="mt-5 flex gap-2">
          {#each modes as m}
            {@const active = currentMode === m.id}
            <button
              class="flex-1 rounded-[var(--radius-kotone-card)] px-3 py-3 text-left ring-1 transition
                {active
                ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan'
                : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
              onclick={() => void selectMode(m.id)}
            >
              <span class="text-lg">{m.icon}</span>
              <p class="mt-1 text-sm font-semibold {active ? 'text-kotone-cyan' : ''}">{m.name}</p>
              <p class="mt-0.5 text-[11px] text-white/50">{m.desc}</p>
            </button>
          {/each}
        </div>

        <div class="mt-4 flex items-center gap-2">
          <span class="text-xs text-white/60">热键</span>
          <span class="rounded bg-white/8 px-2.5 py-1 text-sm font-semibold ring-1 ring-white/15">
            {currentKey}
          </span>
          <button
            class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {capturing
              ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
              : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
            disabled={capturing}
            onclick={() => void startCapture()}
          >
            {capturing ? "请按下热键组合…（Esc 取消）" : "点击录入"}
          </button>
        </div>

        <div class="mt-6 flex items-center justify-between">
          <button
            class="text-xs text-white/45 transition hover:text-white/75"
            onclick={() => (step = 1)}
          >
            ← 上一步
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-sm font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
            disabled={capturing || finishing}
            onclick={() => void finish()}
          >
            完成，开玩！
          </button>
        </div>
      </div>
    {/if}

    <!-- 步骤指示点 -->
    <div class="relative flex justify-center gap-1.5 pb-4">
      {#each [0, 1, 2] as i}
        <span
          class="h-1.5 rounded-full transition-all {i === step
            ? 'w-5 bg-kotone-cyan shadow-glow-cyan'
            : 'w-1.5 bg-white/20'}"
        ></span>
      {/each}
    </div>
  </div>
</div>
