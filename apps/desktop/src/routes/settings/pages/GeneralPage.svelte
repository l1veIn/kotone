<script lang="ts">
  /*
   * 通用页（方向 B 灵魂首屏）：欢迎面板 = 直播间背景 + Kotone 立绘 +
   * 渐变标题 + 三个特性 chip；下方只保留日常高频设置。
   */
  import { onDestroy, onMount } from "svelte";
  import {
    updateSettings,
    listAudioDevices,
    setAudioDevice,
    getElevationStatus,
    restartAsAdmin,
    getHotkeyStatus,
    type AudioDevice,
    type ElevationStatus,
    type HotkeyStatus,
    type InteractionMode,
  } from "../../../lib/ipc";
  import { captureHotkey } from "../../../lib/hotkeyCapture";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import { runtimeStore } from "../../../lib/stores/runtime";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import RuntimeButton from "../../../lib/components/RuntimeButton.svelte";
  import heroGeneral from "../../../assets/brand/hero-general.webp";

  let { onOpenAdvanced }: { onOpenAdvanced: () => void } = $props();

  let devices = $state<AudioDevice[]>([]);
  let elevation = $state<ElevationStatus | null>(null);
  let restarting = $state(false);

  /** 提权重启接力标记：旧进程退出前落 localStorage，新进程检测到已提权后 toast 并清除 */
  const ADMIN_RESTART_FLAG = "kotone:admin-restart-pending";

  onMount(async () => {
    try {
      [devices, elevation] = await Promise.all([listAudioDevices(), getElevationStatus()]);
      if (localStorage.getItem(ADMIN_RESTART_FLAG)) {
        localStorage.removeItem(ADMIN_RESTART_FLAG);
        if (elevation?.elevated) toast(true, "已通过管理员权限运行 ✨");
      }
    } catch (e) {
      toast(false, `加载设备信息失败：${errText(e)}`);
    }
  });

  /** 权限状态轮询：每 3s；页面隐藏时暂停，回到前台立即补一次 */
  onMount(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const refresh = async () => {
      try {
        elevation = await getElevationStatus();
      } catch {
        /* 轮询失败不打扰，下轮再试 */
      }
    };
    const start = () => {
      if (timer === null) timer = setInterval(() => void refresh(), 3000);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        void refresh();
        start();
      }
    };
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  /** UIPI：目标游戏提权且自身未提权 → 显示提权横幅 */
  const needsElevation = $derived(
    elevation !== null && !elevation.elevated && elevation.activeGameElevated === true,
  );

  async function patch(patchObj: Record<string, unknown>, okText: string) {
    try {
      settingsStore.set(await updateSettings(patchObj));
      toast(true, okText);
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
    }
  }

  async function onRestartAsAdmin() {
    restarting = true;
    try {
      // 成功后当前进程直接退出；走到 catch 说明用户取消 UAC 或失败
      localStorage.setItem(ADMIN_RESTART_FLAG, "1");
      await restartAsAdmin();
    } catch (e) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      restarting = false;
      toast(false, errText(e));
    }
  }

  // ---------- 热键 & 交互模式（自快捷键页并入） ----------

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
      hotkeyDraft = $settingsStore?.hotkey.key ?? "CapsLock";
    } catch (e) {
      toast(false, `读取热键状态失败：${errText(e)}`);
    }
  });

  const modes: { id: InteractionMode; name: string; desc: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送" },
    { id: "dictation", name: "录音笔", desc: "点按开始，再点停止" },
    { id: "one-shot", name: "说一句就走", desc: "说完自动停、自动发" },
    { id: "solo", name: "独奏模式", desc: "持续收音，说一句发一句，再点停止" },
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
            key: $settingsStore?.hotkey.key ?? "CapsLock",
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

</script>

<div class="px-6 py-5">
  <!-- 欢迎面板：ord-ui-hero-general 合成视觉（立绘+直播间一张图） -->
  <section class="relative overflow-hidden rounded-[var(--radius-kotone-panel)] ring-1 ring-white/10">
    <img src={heroGeneral} alt="Kotone 在中继站直播间挥手欢迎" class="absolute inset-0 h-full w-full object-cover" />
    <!-- 左侧压暗渐变：保证标题与芯片可读 -->
    <div class="absolute inset-0 bg-gradient-to-r from-kotone-deep/92 via-kotone-deep/55 to-transparent"></div>
    <div class="absolute inset-x-0 bottom-0 h-24 bg-gradient-to-t from-kotone-deep/80 to-transparent"></div>
    <div class="relative flex min-h-64 flex-col justify-between gap-8 px-6 py-6">
      <div>
        <h1 class="text-[26px] leading-tight font-bold">
          想说的话，<br /><span class="kotone-gradient-text">一秒都别等</span>
        </h1>
        <p class="mt-2 text-[12px] leading-relaxed text-white/70">
          嗨，我是 Kotone！你的专属语音助手，<br />随时待命，陪你指挥每一场胜利！
        </p>
      </div>
      <RuntimeButton variant="hero" {onOpenAdvanced} />
    </div>
  </section>

  {#if $settingsStore}
    <!-- 麦克风 -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">麦克风</h2>
      <select
        class="mt-3 w-full rounded-lg bg-white/8 px-2.5 py-2 text-sm ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
        value={$settingsStore.audioDeviceId}
        onchange={async (e) => {
          const id = (e.target as HTMLSelectElement).value;
          try {
            await setAudioDevice(id);
            settingsStore.update((s) => (s ? { ...s, audioDeviceId: id } : s));
            toast(true, "麦克风已切换");
          } catch (err) {
            toast(false, `切换麦克风失败：${errText(err)}`);
          }
        }}
      >
        {#each devices as d}
          <option value={d.id}>{d.name}</option>
        {/each}
      </select>
    </section>

    {#if needsElevation}
      <section class="mt-4 flex items-center gap-3 rounded-xl bg-kotone-pink/12 p-4 ring-1 ring-kotone-pink/45">
        <div class="min-w-0 flex-1">
          <p class="text-sm font-semibold text-kotone-pink">当前游戏需要更高权限才能接收文字</p>
          <p class="mt-1 text-[11px] text-white/55">重新启动琴音后即可继续，其他设置不会丢失。</p>
        </div>
        <button
          class="shrink-0 rounded-lg bg-kotone-pink px-3 py-2 text-xs font-semibold text-white transition hover:brightness-110 disabled:opacity-50"
          disabled={restarting}
          onclick={() => void onRestartAsAdmin()}
        >
          {restarting ? "正在重启…" : "重新启动"}
        </button>
      </section>
    {/if}

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
          placeholder="如 CapsLock / Alt+V"
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

    <!-- 交互模式（ADR-006 四预设，lucide 风格图标） -->
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
            <span
              class="inline-flex h-5 w-5 items-center justify-center {active ? 'text-kotone-cyan' : 'text-white/60'}"
              aria-hidden="true"
            >
              {#if m.id === "push-to-talk"}
                <!-- lucide: mic -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>
              {:else if m.id === "dictation"}
                <!-- lucide: circle-dot（录音） -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3" fill="currentColor" stroke="none"/></svg>
              {:else if m.id === "one-shot"}
                <!-- lucide: rocket -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>
              {:else}
                <!-- lucide: music（独奏） -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4.5 w-4.5"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>
              {/if}
            </span>
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

    <!-- 悬浮窗：普通用户只需要决定何时出现、放在哪里、是否穿透。 -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">悬浮窗</h2>
      <p class="mt-2 text-[10px] font-semibold tracking-wide text-white/40">什么时候出现</p>
      <div class="mt-1.5 grid grid-cols-2 gap-2">
        {#each [
          { id: "on_demand", name: "用时浮现", desc: "平时隐藏，说话时出现，发完自动隐藏" },
          { id: "always", name: "常驻", desc: "启动即显示，停止才隐藏" },
        ] as opt}
          {@const selected = ($settingsStore.overlay?.visibility ?? "on_demand") === opt.id}
          <button
            class="rounded-[var(--radius-kotone-card)] px-3 py-2.5 text-left ring-1 transition
              {selected
              ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan'
              : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
            onclick={() =>
              void patch(
                { overlay: { visibility: opt.id } },
                opt.id === "always" ? "悬浮窗已切换：常驻" : "悬浮窗已切换：用时浮现",
              )}
          >
            <p class="text-sm font-semibold {selected ? 'text-kotone-cyan' : ''}">{opt.name}</p>
            <p class="mt-0.5 text-[11px] text-white/50">{opt.desc}</p>
          </button>
        {/each}
      </div>

      <p class="mt-4 text-[10px] font-semibold tracking-wide text-white/40">固定位置</p>
      <div class="mt-1.5 grid grid-cols-4 gap-2">
        {#each [
          { id: "auto", name: "智能" },
          { id: "top_left", name: "左上" },
          { id: "top_center", name: "顶部" },
          { id: "top_right", name: "右上" },
          { id: "center", name: "中央" },
          { id: "bottom_left", name: "左下" },
          { id: "bottom_center", name: "底部" },
          { id: "bottom_right", name: "右下" },
        ] as opt}
          {@const selected = ($settingsStore.overlay.position ?? "auto") === opt.id}
          <button
            class="rounded-lg px-2 py-2 text-center text-xs font-semibold ring-1 transition
              {selected
              ? 'bg-kotone-cyan/12 text-kotone-cyan ring-kotone-cyan/60'
              : 'bg-white/5 text-white/60 ring-white/10 hover:bg-white/10'}"
            onclick={() =>
              void patch(
                { overlay: { position: opt.id } },
                `悬浮窗位置已切换：${opt.name}`,
              )}
          >
            {opt.name}
          </button>
        {/each}
      </div>
      {#if $settingsStore.overlay.position === "custom"}
        <p class="mt-2 text-[11px] text-kotone-cyan/75">当前使用你上次拖动后的位置。</p>
      {/if}

      <div class="mt-4 flex flex-col gap-4 rounded-lg bg-white/4 p-3 ring-1 ring-white/8">
        <Toggle
          checked={!$settingsStore.overlay.clickThrough}
          label="允许鼠标操作和拖动悬浮窗"
          desc="开启时可拖动并操作按钮；关闭后自动锁定位置，鼠标会穿透到游戏"
          onchange={(v) =>
            void patch(
              { overlay: { draggable: v, clickThrough: !v } },
              v ? "悬浮窗已允许操作和拖动" : "悬浮窗已锁定并开启鼠标穿透",
            )}
        />
      </div>
    </section>





  {/if}
</div>
