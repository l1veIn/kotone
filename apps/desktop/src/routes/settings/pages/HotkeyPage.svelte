<script lang="ts">
  /*
   * 快捷键页：交互模式四选卡（interactionMode）+ vadSilenceMs 滑块（one-shot / solo）+
   * 热键录入捕获（ADR-006）+ 热键后端与注册状态。
   */
  import { onDestroy, onMount } from "svelte";
  import {
    updateSettings,
    getHotkeyStatus,
    type HotkeyStatus,
    type InteractionMode,
  } from "../../../lib/ipc";
  import { captureHotkey } from "../../../lib/hotkeyCapture";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import { runtimeStore } from "../../../lib/stores/runtime";

  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  /** 热键编辑草稿（保存前不动后端） */
  let hotkeyDraft = $state("");
  /** 热键录入捕获中（「点击录入」按钮态） */
  let capturing = $state(false);
  /** 进行中捕获的 cleanup（组件销毁兜底取消） */
  let captureCleanup: (() => void) | null = null;

  onMount(async () => {
    try {
      hotkeyStatus = await getHotkeyStatus();
      hotkeyDraft = $settingsStore?.hotkey.key ?? "F8";
    } catch (e) {
      toast(false, `读取热键状态失败：${errText(e)}`);
    }
  });

  const modes: { id: InteractionMode; name: string; desc: string; icon: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送", icon: "🎙️" },
    { id: "dictation", name: "录音笔", desc: "点按开始，再点停止", icon: "⏺️" },
    { id: "one-shot", name: "说一句就走", desc: "说完自动停、自动发", icon: "🚀" },
    { id: "solo", name: "独奏模式", desc: "持续收音，说一句发一句，再点停止", icon: "🎹" },
  ];

  const currentMode = $derived($settingsStore?.interactionMode ?? null);
  const vadMs = $derived($settingsStore?.vadSilenceMs ?? 700);

  async function onModeSelect(id: InteractionMode) {
    try {
      // 预设与 hotkey.mode 同步落盘：壳侧注册用 effective_hotkey_mode 推导，
      // 这里保持旧字段一致，避免设置页其它处读到脱节的 mode。
      // 对讲机 = 按住（hold），录音笔 / 说一句就走 = 点按（toggle）。
      settingsStore.set(
        await updateSettings({
          interactionMode: id,
          hotkey: {
            key: $settingsStore?.hotkey.key ?? "F8",
            mode: id === "push-to-talk" ? "hold" : "toggle",
          },
        }),
      );
      const m = modes.find((x) => x.id === id);
      toast(true, `交互模式已切换：${m?.name ?? id}`);
    } catch (e) {
      toast(false, `切换模式失败：${errText(e)}`);
    }
  }

  async function onVadChange(e: Event) {
    const vadSilenceMs = Number((e.target as HTMLInputElement).value);
    try {
      settingsStore.set(await updateSettings({ vadSilenceMs }));
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function saveHotkey() {
    const key = hotkeyDraft.trim();
    if (!key) {
      toast(false, "热键不能为空");
      return;
    }
    try {
      // 热键变更后端自动重注册（lib.rs update_settings）；模式沿用当前值
      settingsStore.set(
        await updateSettings({ hotkey: { key, mode: $settingsStore?.hotkey.mode ?? "toggle" } }),
      );
      hotkeyDraft = $settingsStore?.hotkey.key ?? key;
      toast(true, `热键已保存并生效：${$settingsStore?.hotkey.key}`);
    } catch (e) {
      toast(false, `保存热键失败：${errText(e)}`);
    } finally {
      try {
        hotkeyStatus = await getHotkeyStatus();
      } catch {
        /* 状态刷新失败不覆盖保存反馈 */
      }
    }
  }

  /**
   * 「点击录入」：LL 钩子捕获下一个按键组合（共享 helper 见 lib/hotkeyCapture.ts）。
   */
  async function startCapture() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((r) => {
      capturing = false;
      captureCleanup = null;
      if (r.kind === "combo") {
        hotkeyDraft = r.combo;
        void saveHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  // 组件销毁时兜底取消进行中的捕获（避免钩子停在捕获模式）
  onDestroy(() => {
    captureCleanup?.();
  });

  async function onHotkeyBackendChange(e: Event) {
    const hotkeyBackend = (e.target as HTMLSelectElement).value;
    try {
      settingsStore.set(await updateSettings({ hotkeyBackend }));
      toast(true, "热键后端已切换并重注册");
    } catch (err) {
      toast(false, `切换热键后端失败：${errText(err)}`);
    } finally {
      try {
        hotkeyStatus = await getHotkeyStatus();
      } catch {
        /* 状态刷新失败不覆盖切换反馈 */
      }
    }
  }

  /** 当前生效后端的展示名 */
  const backendLabel = $derived(
    hotkeyStatus === null
      ? "检测中…"
      : hotkeyStatus.backend === "llhook"
        ? "LL 钩子（游戏前台可用）"
        : hotkeyStatus.backend === "register"
          ? "RegisterHotKey（系统热键）"
          : "未注册",
  );
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">快捷键</h1>
  <p class="mt-0.5 text-[11px] text-white/45">说话的方式，由你定</p>

  <!-- 交互模式（ADR-006 三预设） -->
  <section class="kotone-panel mt-4 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">交互模式</h2>
    <div class="mt-3 flex gap-2">
      {#each modes as m}
        {@const active = currentMode === m.id}
        <button
          class="flex-1 rounded-[var(--radius-kotone-card)] px-3 py-3 text-left ring-1 transition
            {active
            ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan'
            : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
          onclick={() => void onModeSelect(m.id)}
        >
          <span class="text-lg">{m.icon}</span>
          <p class="mt-1 text-sm font-semibold {active ? 'text-kotone-cyan' : ''}">{m.name}</p>
          <p class="mt-0.5 text-[11px] text-white/50">{m.desc}</p>
        </button>
      {/each}
    </div>
    {#if currentMode === null}
      <p class="mt-2 text-[11px] text-white/40">
        当前为兼容模式（由热键触发方式 + 发送行为旧字段推导）；点选上方卡片即切换为预设模式。
      </p>
    {/if}

    {#if currentMode === "one-shot" || currentMode === "solo"}
      <!-- one-shot / solo 专属：VAD 静音判停时长（solo 按该阈值切分每一句） -->
      <div class="mt-4 rounded-lg bg-white/5 p-3 ring-1 ring-white/10">
        <div class="flex items-center justify-between">
          <label class="text-xs text-white/70" for="vad-slider">静音判停时长</label>
          <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
            {vadMs} ms
          </span>
        </div>
        <input
          id="vad-slider"
          type="range"
          min="200"
          max="5000"
          step="100"
          value={vadMs}
          onchange={(e) => void onVadChange(e)}
          class="mt-2 w-full accent-kotone-cyan"
        />
        <p class="mt-1 text-[11px] text-white/40">
          {#if currentMode === "solo"}
            每句之间静音超过该时长即切分并发送，随后继续监听下一句（200–5000ms，需要 VAD 模型就绪）。
          {:else}
            说完话后静音超过该时长即自动结束并发送（200–5000ms，需要 VAD 模型就绪）。
          {/if}
        </p>
      </div>
    {/if}
  </section>

  <!-- 热键 -->
  <section class="kotone-panel mt-4 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">热键</h2>
    {#if $runtimeStore && $runtimeStore.phase !== "running"}
      <p class="mt-2 rounded-lg bg-white/5 px-2.5 py-1.5 text-[11px] text-white/50 ring-1 ring-white/10">
        运行时已停止：热键未注册（按键无反应），点标题栏「启动」后按当前配置生效；
        这里的录入与保存会立即写入配置。
      </p>
    {/if}
    <div class="mt-3 flex items-center gap-2">
      <input
        id="hotkey-input"
        bind:value={hotkeyDraft}
        disabled={capturing}
        class="w-32 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
        placeholder="如 F8 / Alt+V"
        spellcheck="false"
      />
      <button
        class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {capturing
          ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
          : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
        disabled={capturing}
        onclick={() => void startCapture()}
      >
        {capturing ? "请按下热键组合…（Esc 取消）" : "点击录入"}
      </button>
      <button
        class="rounded-lg bg-kotone-cyan px-3 py-1.5 text-xs font-semibold text-kotone-deep transition hover:shadow-glow-cyan hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={capturing}
        onclick={() => void saveHotkey()}
      >
        保存并生效
      </button>
      <span class="text-[11px] text-white/40">当前：{$settingsStore?.hotkey.key ?? "…"}</span>
    </div>
    <div class="mt-3 flex items-center gap-2">
      <label class="text-xs text-white/60" for="hotkey-backend">热键后端</label>
      <select
        id="hotkey-backend"
        class="rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
        value={$settingsStore?.hotkeyBackend ?? "auto"}
        onchange={(e) => void onHotkeyBackendChange(e)}
      >
        <option value="auto">自动（优先 LL 钩子）</option>
        <option value="llhook">LL 钩子</option>
        <option value="register">RegisterHotKey</option>
      </select>
      <span class="text-[11px] text-white/40">当前生效：{backendLabel}</span>
    </div>
    <p class="mt-2 text-[11px] text-white/40">
      部分游戏前台时系统热键（RegisterHotKey）收不到按键；LL 钩子（WH_KEYBOARD_LL）可覆盖该场景，命中时会吞掉热键避免触发游戏内同键绑定。
    </p>
    {#if hotkeyStatus?.error}
      <div class="mt-3 rounded-lg bg-kotone-pink/15 p-2.5 ring-1 ring-kotone-pink/50">
        <p class="text-xs font-medium text-kotone-pink">
          热键注册失败，可能被其他程序或其他 Kotone 实例占用
        </p>
        <p class="mt-0.5 text-[11px] break-all text-white/50">{hotkeyStatus.error}</p>
        <button
          class="mt-2 rounded-lg bg-kotone-pink/80 px-2.5 py-1 text-xs font-semibold text-white transition hover:brightness-110 active:scale-95"
          onclick={() => void saveHotkey()}
        >
          重新注册
        </button>
      </div>
    {/if}
  </section>
</div>
