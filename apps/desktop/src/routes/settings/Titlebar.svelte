<script lang="ts">
  /*
   * 标准标题栏（main 窗 decorations:false，方向 B 富化，高 48px）：
   * - 左：Kotone 小圆图标（青光晕）+ 名称 + 状态区两行（上行：状态灯 + 相位文字；
   *   下行：引擎 · 模型 · 交互模式）；
   * - 右：运行中显示 CPU/内存占用（2s 轮询）+ 启动/停止/重启生效按钮
   *   + 标准窗口控制两键（min / close，最右上角；conf 已设 maximizable:false）；
   * - 拖拽：非按钮区 mousedown 由 startDragging() 接管；
   * - close = 弹确认框：「退出」彻底退出进程，「最小化到托盘」保持隐藏语义。
   */
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { exit } from "@tauri-apps/plugin-process";
  import { getResourceUsage, isTauri } from "../../lib/ipc";
  import { runtimeStore } from "../../lib/stores/runtime";
  import { settingsStore, toast, errText } from "../../lib/stores/ui";
  import ExitPrompt from "../../lib/components/ExitPrompt.svelte";
  import RuntimeButton from "../../lib/components/RuntimeButton.svelte";
  import iconSrc from "../../assets/brand/icon-src.png";

  let { onOpenAdvanced }: { onOpenAdvanced: () => void } = $props();

  const modeNames: Record<string, string> = {
    "push-to-talk": "对讲机",
    dictation: "录音笔",
    "one-shot": "说一句就走",
    solo: "独奏模式",
  };
  const stageNames: Record<string, string> = {
    warmup: "正在加载引擎…",
    hotkey: "正在注册热键…",
    overlay: "正在显示悬浮窗…",
    unload: "正在卸载引擎…",
  };

  const rt = $derived($runtimeStore);
  let showExitPrompt = $state(false);
  let exiting = $state(false);

  /** 运行中的资源占用文本（空 = 非 running 或尚未采样成功） */
  let resourceText = $state("");

  async function pollResourceUsage() {
    try {
      const u = await getResourceUsage();
      resourceText = `CPU ${u.cpuPercent.toFixed(1)}% · 内存 ${Math.round(u.memoryBytes / 1024 / 1024)} MB`;
    } catch {
      /* 采样失败保留旧值 */
    }
  }

  // running 期间每 2s 轮询资源占用（CPU% 依赖后端两次采样间隔）；离开 running 或销毁时清除
  $effect(() => {
    if (rt?.phase !== "running") {
      resourceText = "";
      return;
    }
    void pollResourceUsage();
    const timer = setInterval(() => void pollResourceUsage(), 2000);
    return () => clearInterval(timer);
  });

  const phaseLabel = $derived(
    !rt
      ? "状态未知"
      : rt.phase === "stopped"
        ? "已停止"
        : rt.phase === "running"
          ? rt.restartNeeded
            ? "运行中 · 变更待重启"
            : "运行中"
          : (stageNames[rt.stage ?? ""] ?? "处理中…"),
  );

  /** 标题栏只呈现用户任务状态；引擎与模型详情收纳在高级页。 */
  const hotkeyLabel = $derived($settingsStore?.hotkey.key ?? "—");
  const statusLine = $derived(
    !rt
      ? `快捷键：${hotkeyLabel} · 状态未知`
      : rt.phase === "stopped"
        ? `快捷键：${hotkeyLabel} · 未注册 · 点右侧「启动」开始说话`
        : rt.phase === "running"
          ? `快捷键：${hotkeyLabel} · 已注册 · ${
              rt.interactionMode ? (modeNames[rt.interactionMode] ?? "自定义模式") : "自定义模式"
            }`
          : `快捷键：${hotkeyLabel} · ${phaseLabel}`,
  );

  /** 非按钮区按下即整窗拖拽（按钮自身 mousedown 不触发） */
  function onDragMouseDown(e: MouseEvent) {
    if (!isTauri) return;
    if ((e.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }

  async function minimize() {
    if (!isTauri) return;
    try {
      await getCurrentWindow().minimize();
    } catch {
      /* 最小化失败静默（窗口可能已销毁） */
    }
  }

  /** 关闭：展示 Kotone 自绘确认弹窗。 */
  function close() {
    if (!isTauri) return;
    showExitPrompt = true;
  }

  async function minimizeToTray() {
    try {
      await getCurrentWindow().hide();
      showExitPrompt = false;
    } catch (e) {
      toast(false, `最小化到托盘失败：${errText(e)}`);
    }
  }

  async function quitApp() {
    if (exiting) return;
    exiting = true;
    try {
      await exit(0);
    } catch (e) {
      exiting = false;
      toast(false, `退出失败：${errText(e)}`);
    }
  }
</script>

<!-- 标题栏整行可拖拽：非按钮区 mousedown 由 startDragging 接管（按钮已排除） -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="relative z-20 flex h-12 shrink-0 items-center justify-between border-b border-white/8 bg-kotone-panel/60 select-none"
  onmousedown={onDragMouseDown}
>
  <!-- 克制的背景光晕（方向 B 霓虹语言，标题栏尺度收敛） -->
  <div
    class="pointer-events-none absolute -top-12 left-1/4 h-20 w-56 rounded-full bg-kotone-violet/15 blur-3xl"
  ></div>
  <div
    class="pointer-events-none absolute -top-10 right-1/3 h-16 w-44 rounded-full blur-3xl transition-colors duration-500 {rt?.phase ===
    'running'
      ? 'bg-kotone-cyan/12'
      : 'bg-kotone-pink/8'}"
  ></div>

  <!-- 左：品牌 + 状态信息（h-12 下两行压缩排布：行距 gap-px + leading 收紧） -->
  <div class="relative flex min-w-0 flex-1 items-center gap-2.5 px-4">
    <img
      src={iconSrc}
      alt=""
      class="h-8 w-8 shrink-0 rounded-full ring-1 ring-kotone-cyan/70 shadow-glow-cyan object-cover"
    />
    <p class="shrink-0 text-xs font-bold tracking-wide">
      Kotone <span class="font-medium text-white/55">琴音</span>
    </p>
    <div class="h-5 w-px shrink-0 bg-white/12"></div>
    <!-- 状态区两行：上行 = 状态灯 + 相位文字；下行 = 引擎 · 模型 · 交互模式 -->
    <div class="flex min-w-0 flex-col justify-center gap-px">
      <div class="flex items-center gap-2 leading-none">
        <span class="relative flex h-2 w-2 shrink-0">
          <span
            class="h-2 w-2 rounded-full transition-colors {!rt || rt.phase === 'stopped'
              ? 'bg-white/25'
              : rt.phase === 'running'
                ? 'bg-kotone-cyan shadow-glow-cyan animate-pulse'
                : 'bg-kotone-pink shadow-glow-pink animate-ping'}"
          ></span>
          {#if rt?.restartNeeded}
            <span
              class="absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full bg-yellow-400 ring-1 ring-kotone-panel"
              title="配置已变更，需重启生效"
            ></span>
          {/if}
        </span>
        <span
          class="text-[11px] leading-tight font-semibold {rt?.phase === 'running'
            ? 'text-kotone-cyan'
            : rt && rt.phase !== 'stopped'
              ? 'text-kotone-pink'
              : 'text-white/45'}"
        >
          {phaseLabel}
        </span>
      </div>
      {#if statusLine}
        <p class="truncate pl-4 text-[10px] leading-tight text-white/40" title={statusLine}>
          {statusLine}
        </p>
      {/if}
    </div>
  </div>

  <!-- 右：资源占用（running 时）+ 启动/停止 + 标准窗口控制两键 -->
  <div class="relative flex h-full shrink-0 items-center">
    {#if rt?.phase === "running" && resourceText}
      <span class="mr-2 text-[10px] text-white/40 tabular-nums" title="Kotone 进程资源占用">
        {resourceText}
      </span>
    {/if}
    <RuntimeButton variant="titlebar" {onOpenAdvanced} />
    <div class="h-4 w-px bg-white/10"></div>
    <button
      class="flex h-full w-12 items-center justify-center text-white/55 transition hover:bg-white/10 hover:text-white"
      title="最小化"
      onclick={() => void minimize()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M5 12h14" stroke-linecap="round" /></svg>
    </button>
    <button
      class="flex h-full w-12 items-center justify-center text-white/55 transition hover:bg-red-500 hover:text-white"
      title="关闭"
      onclick={() => void close()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" /></svg>
    </button>
  </div>
</header>

{#if showExitPrompt}
  <ExitPrompt
    busy={exiting}
    onExit={() => void quitApp()}
    onMinimize={() => void minimizeToTray()}
  />
{/if}
