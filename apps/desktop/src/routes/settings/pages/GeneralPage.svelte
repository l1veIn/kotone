<script lang="ts">
  /*
   * 通用页（方向 B 灵魂首屏）：欢迎面板 = 直播间背景 + Kotone 立绘 +
   * 渐变标题 + 三个特性 chip；下方为通用设置（麦克风 / 发送行为 / 录档 / 权限）。
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

  let devices = $state<AudioDevice[]>([]);
  let elevation = $state<ElevationStatus | null>(null);
  let restarting = $state(false);

  onMount(async () => {
    try {
      [devices, elevation] = await Promise.all([listAudioDevices(), getElevationStatus()]);
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
      await restartAsAdmin();
    } catch (e) {
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

    <!-- 发送行为 -->
    <section class="kotone-panel mt-4 flex flex-col gap-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">发送行为</h2>
      <Toggle
        checked={$settingsStore.autoSend}
        label="转写完成后直接发送"
        desc="关闭时先弹出预览，确认后再发送"
        onchange={(v) => void patch({ autoSend: v }, v ? "已开启转写后直接发送" : "已改为预览确认后发送")}
      />
      <Toggle
        checked={$settingsStore.evalRecording}
        label="评测录档"
        desc="保存每次会话的音频与指标，用于引擎对比评测"
        onchange={(v) => void patch({ evalRecording: v }, v ? "已开启评测录档" : "已关闭评测录档")}
      />
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

    <!-- 权限（UIPI 提权方案，docs/development.md §10 R-1） -->
    <section class="kotone-panel mt-4 p-4">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">权限</h2>
      <div class="mt-3 flex items-center justify-between">
        <span class="text-sm">当前运行权限</span>
        <span
          class="rounded px-2 py-0.5 text-xs font-semibold {elevation?.elevated
            ? 'bg-kotone-cyan/20 text-kotone-cyan'
            : 'bg-white/10 text-white/60'}"
        >
          {elevation === null ? "检测中…" : elevation.elevated ? "管理员" : "普通用户"}
        </span>
      </div>

      {#if needsElevation}
        <div class="mt-3 rounded-lg bg-kotone-pink/15 p-3 ring-1 ring-kotone-pink/60">
          <p class="text-sm font-semibold text-kotone-pink">
            检测到游戏以管理员运行，Kotone 需要同等权限才能发送
          </p>
          <p class="mt-1 text-[11px] leading-relaxed text-white/60">
            Windows UIPI 会拦截低权限进程发往高权限游戏的模拟输入。重启后会弹出一次 UAC 确认。
          </p>
        </div>
      {/if}

      {#if elevation && !elevation.elevated}
        <button
          class="mt-3 w-full rounded-lg px-3 py-2 text-sm font-semibold transition active:scale-95 disabled:opacity-50 {needsElevation
            ? 'bg-kotone-pink text-white shadow-glow-pink hover:brightness-110'
            : 'bg-white/10 text-white/80 hover:bg-white/20'}"
          disabled={restarting}
          onclick={() => void onRestartAsAdmin()}
        >
          {restarting ? "正在重启…" : "以管理员身份重启"}
        </button>
        {#if !needsElevation}
          <p class="mt-2 text-[11px] text-white/40">
            当前为普通权限。若目标游戏以管理员运行，发送会被系统拦截，此处会提示提权。
          </p>
        {/if}
      {/if}

      <div class="mt-4">
        <Toggle
          checked={$settingsStore.runAsAdminOnStart}
          label="启动时自动以管理员运行"
          desc="每次启动若未提权则自动重启并弹一次 UAC；取消 UAC 则本次按普通权限运行"
          onchange={(v) =>
            void patch(
              { runAsAdminOnStart: v },
              v ? "已开启启动时自动提权（每次启动弹一次 UAC）" : "已关闭启动时自动提权",
            )}
        />
      </div>
      {#if $settingsStore.runAsAdminOnStart && elevation && !elevation.elevated}
        <p class="mt-2 text-[11px] text-white/45">
          已开启，将在下次启动时生效 ·
          <button
            class="text-kotone-cyan underline underline-offset-2 transition hover:brightness-125 disabled:opacity-50"
            disabled={restarting}
            onclick={() => void onRestartAsAdmin()}
          >
            立即以管理员身份重启
          </button>
        </p>
      {/if}
    </section>
  {/if}
</div>
