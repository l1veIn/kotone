<script lang="ts">
  /*
   * 标准标题栏（main 窗 decorations:false，方向 B 富化，高 48px）：
   * - 左：Kotone 小圆图标（青光晕）+ 名称 + 状态区两行（上行：状态灯 + 相位文字；
   *   下行：引擎 · 模型 · 交互模式）；
   * - 右：启动/停止/重启生效按钮 + 标准窗口控制三键（min / max|还原 / close，最右上角）；
   * - 拖拽：非按钮区 mousedown 由 startDragging() 接管；max/还原图标由 onResized 驱动；
   * - close = 隐藏到托盘（与 CloseRequested 拦截语义一致；退出走托盘菜单）。
   */
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { isTauri, startRuntime, stopRuntime } from "../../lib/ipc";
  import { runtimeStore } from "../../lib/stores/runtime";
  import { toast, toastInfo, pushToast, errText } from "../../lib/stores/ui";
  import iconSrc from "../../assets/brand/icon-src.png";

  let { onOpenAdvanced }: { onOpenAdvanced: () => void } = $props();

  const modeNames: Record<string, string> = {
    "push-to-talk": "对讲机",
    dictation: "录音笔",
    "one-shot": "说一句就走",
  };
  const stageNames: Record<string, string> = {
    warmup: "正在加载引擎…",
    hotkey: "正在注册热键…",
    overlay: "正在显示悬浮窗…",
    unload: "正在卸载引擎…",
  };

  const rt = $derived($runtimeStore);
  const busy = $derived(rt?.phase === "starting" || rt?.phase === "stopping");
  /** 防连点（busy 已由相位覆盖，acting 覆盖事件未回的瞬间） */
  let acting = $state(false);
  let isMaximized = $state(false);

  onMount(() => {
    if (!isTauri) return;
    let un: (() => void) | undefined;
    void (async () => {
      const win = getCurrentWindow();
      isMaximized = await win.isMaximized();
      un = await win.onResized(async () => {
        isMaximized = await win.isMaximized();
      });
    })();
    return () => un?.();
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
  const statusLine = $derived(
    !rt
      ? ""
      : rt.phase === "stopped"
        ? "热键未注册 · 点右侧「启动」开始说话"
        : `语音输入已就绪 · ${
            rt.interactionMode ? (modeNames[rt.interactionMode] ?? "自定义模式") : "自定义模式"
          }`,
  );

  async function onMainButton() {
    if (!rt || busy || acting) return;
    acting = true;
    const restarting = rt.phase === "running" && rt.restartNeeded;
    try {
      if (rt.phase === "running" && !rt.restartNeeded) {
        await stopRuntime();
        toastInfo("已停止，热键已注销");
      } else {
        // stopped → 启动；running + restartNeeded → 重启（stop+start，壳侧编排）
        await startRuntime();
        pushToast(
          "success",
          restarting ? "已按新配置重启，Kotone 运行中 ✨" : "模型已加载，Kotone 已启动 ✨",
        );
      }
    } catch (e) {
      const message = errText(e);
      toast(false, message);
      if (/模型.*未下载|未就绪|模型文件|recognizer 创建失败/.test(message)) {
        onOpenAdvanced();
      }
    } finally {
      acting = false;
    }
  }

  /** 非按钮区按下即整窗拖拽（按钮自身 mousedown 不触发） */
  function onDragMouseDown(e: MouseEvent) {
    if (!isTauri) return;
    if ((e.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }

  async function minimize() {
    if (isTauri) await getCurrentWindow().minimize();
  }

  async function toggleMaximize() {
    if (isTauri) await getCurrentWindow().toggleMaximize();
  }

  /** 关闭 = 隐藏到托盘（与 CloseRequested 拦截语义一致；退出走托盘菜单） */
  async function close() {
    if (isTauri) await getCurrentWindow().hide();
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
      class="h-6 w-6 shrink-0 rounded-full ring-1 ring-kotone-cyan/70 shadow-glow-cyan object-cover"
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

  <!-- 右：启动/停止 + 标准窗口控制三键 -->
  <div class="relative flex h-full shrink-0 items-center">
    <button
      class="mr-2 flex h-7 items-center gap-1.5 rounded-[var(--radius-kotone-card)] px-3 text-[11px] font-bold transition active:scale-95 disabled:cursor-not-allowed {busy ||
      acting
        ? 'bg-white/10 text-white/50'
        : !rt || rt.phase === 'stopped'
          ? 'bg-kotone-pink text-white shadow-glow-pink-lg hover:brightness-110'
          : rt.restartNeeded
            ? 'bg-yellow-400/90 text-kotone-deep shadow-[0_0_14px_rgba(250,204,21,0.4)] hover:brightness-110'
            : 'bg-kotone-cyan/15 text-kotone-cyan ring-1 ring-kotone-cyan/50 hover:bg-kotone-cyan/25 hover:shadow-glow-cyan'}"
      disabled={busy || acting || !rt}
      onclick={() => void onMainButton()}
    >
      {#if busy || acting}
        <span class="h-3 w-3 animate-spin rounded-full border-2 border-white/25 border-t-white/80"></span>
        {rt?.phase === "stopping" ? "停止中" : "启动中"}
      {:else if !rt || rt.phase === "stopped"}
        ▶ 启动
      {:else if rt.restartNeeded}
        重启生效
      {:else}
        ■ 停止
      {/if}
    </button>
    <div class="h-4 w-px bg-white/10"></div>
    <button
      class="flex h-full w-12 items-center justify-center text-white/55 transition hover:bg-white/10 hover:text-white"
      title="最小化"
      onclick={() => void minimize()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M5 12h14" stroke-linecap="round" /></svg>
    </button>
    <button
      class="flex h-full w-12 items-center justify-center text-white/55 transition hover:bg-white/10 hover:text-white"
      title={isMaximized ? "还原" : "最大化"}
      onclick={() => void toggleMaximize()}
    >
      {#if isMaximized}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-3.5 w-3.5"><rect x="7" y="7" width="11" height="11" rx="1.5" /><path d="M17 7V5.5A1.5 1.5 0 0 0 15.5 4H6A1.5 1.5 0 0 0 4.5 5.5V15A1.5 1.5 0 0 0 6 16.5H7" stroke-linecap="round" transform="translate(0.5 0.5)" /></svg>
      {:else}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-3.5 w-3.5"><rect x="5.5" y="5.5" width="13" height="13" rx="1.5" /></svg>
      {/if}
    </button>
    <button
      class="flex h-full w-12 items-center justify-center text-white/55 transition hover:bg-red-500 hover:text-white"
      title="关闭（隐藏到托盘）"
      onclick={() => void close()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" /></svg>
    </button>
  </div>
</header>
