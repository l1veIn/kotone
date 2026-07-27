<script lang="ts">
  /*
   * 通用页（方向 B 灵魂首屏）：欢迎面板 = 直播间背景 + Kotone 立绘 +
   * 渐变标题 + 三个特性 chip；下方只保留日常高频设置。
   */
  import { onMount } from "svelte";
  import {
    updateSettings,
    listAudioDevices,
    setAudioDevice,
    getElevationStatus,
    restartAsAdmin,
    type AudioDevice,
    type ElevationStatus,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import relayBg from "../../../assets/brand/relay-room-bg.png";
  import cutout from "../../../assets/brand/kotone-cutout.png";

  let { onOpenOnboarding }: { onOpenOnboarding: () => void } = $props();

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

  const chips = [
    { icon: "⚡", title: "极速响应", desc: "毫秒级识别" },
    { icon: "🎯", title: "高精度识别", desc: "游戏术语优化" },
    { icon: "🛡️", title: "隐私安全", desc: "本地优先处理" },
  ];
</script>

<div class="px-6 py-5">
  <!-- 欢迎面板：直播间氛围背景 + 立绘 + 渐变标题 -->
  <section class="relative overflow-hidden rounded-[var(--radius-kotone-panel)] ring-1 ring-white/10">
    <img src={relayBg} alt="" class="absolute inset-0 h-full w-full object-cover" />
    <!-- 暗色渐变遮罩：保证文字对比度 -->
    <div class="absolute inset-0 bg-gradient-to-r from-kotone-deep/40 via-kotone-deep/70 to-kotone-deep/92"></div>
    <div class="relative flex items-end gap-2 px-6 pt-6">
      <img
        src={cutout}
        alt="Kotone 立绘"
        class="h-56 shrink-0 object-contain drop-shadow-[0_0_24px_rgba(0,229,255,0.25)]"
      />
      <div class="min-w-0 flex-1 pb-5 pl-2">
        <h1 class="text-[26px] leading-tight font-bold">
          想说的话，<br /><span class="kotone-gradient-text">一秒都别等</span>
        </h1>
        <p class="mt-2 text-[12px] leading-relaxed text-white/65">
          嗨，我是 Kotone！你的专属语音助手，<br />随时待命，陪你指挥每一场胜利！
        </p>
      </div>
    </div>
    <div class="relative flex gap-2.5 px-6 pb-5 pl-6">
      {#each chips as chip}
        <div class="kotone-card flex flex-1 items-center gap-2 px-3 py-2">
          <span class="text-base">{chip.icon}</span>
          <span>
            <span class="block text-[12px] font-semibold">{chip.title}</span>
            <span class="block text-[10px] text-white/45">{chip.desc}</span>
          </span>
        </div>
      {/each}
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

    <!-- 悬浮窗：普通用户只需要决定何时出现、放在哪里、是否穿透。 -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">悬浮窗</h2>
      <p class="mt-2 text-[10px] font-semibold tracking-wide text-white/40">什么时候出现</p>
      <div class="mt-1.5 grid grid-cols-2 gap-2">
        {#each [
          { id: "always", name: "常驻", desc: "启动即显示，停止才隐藏" },
          { id: "on_demand", name: "用时浮现", desc: "平时隐藏，说话时出现，发完自动隐藏" },
        ] as opt}
          {@const selected = ($settingsStore.overlay?.visibility ?? "always") === opt.id}
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
          checked={$settingsStore.overlay.draggable}
          label="允许拖动悬浮窗"
          desc="按住悬浮窗空白处拖动，松开后自动记住位置"
          onchange={(v) =>
            void patch({ overlay: { draggable: v } }, v ? "悬浮窗已允许拖动" : "悬浮窗位置已锁定")}
        />
        <Toggle
          checked={$settingsStore.overlay.clickThrough}
          label="鼠标点击穿透"
          desc="鼠标操作会直接落到游戏；需要调整位置时先在这里关闭"
          onchange={(v) =>
            void patch(
              { overlay: { clickThrough: v } },
              v ? "已开启鼠标点击穿透" : "已关闭鼠标点击穿透",
            )}
        />
      </div>
    </section>

    <!-- 运行时（「启动」开关，core runtime 状态机） -->
    <section class="kotone-panel mt-4 flex flex-col gap-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">运行时</h2>
      <Toggle
        checked={$settingsStore.ui.autoStart}
        label="启动 Kotone 后自动开始运行"
        desc="自动加载识别引擎、注册热键并显示悬浮窗；关闭时需手动点标题栏「启动」"
        onchange={(v) =>
          void patch({ ui: { autoStart: v } }, v ? "已开启自动启动" : "已关闭自动启动，需手动点「启动」")}
      />
    </section>

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

  {/if}
</div>
