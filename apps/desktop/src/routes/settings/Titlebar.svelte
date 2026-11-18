<script lang="ts">
  /*
   * 自绘标题栏（main 窗 decorations:false，高 40px，与侧边栏同底色，参照 DeepSeek Harness 的
   * Windows 标题行）：
   * - 左：侧边栏开关 + Kotone 图标与名称；状态胶囊（状态灯 + 相位 + 快捷键状态）；
   * - 右：运行中的 CPU / 内存（2s 轮询）+ 启动 / 停止 / 重启生效 + 最小化 / 最大化（还原）/ 关闭；
   * - 非按钮区按下拖动窗口，双击切换最大化；
   * - 关闭 = 弹确认框：「退出」彻底退出进程，「最小化到托盘」保持隐藏语义。
   */
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { exit } from "@tauri-apps/plugin-process";
  import { Copy, Minus, PanelLeft, Square, X } from "@lucide/svelte";
  import { getResourceUsage, isTauri } from "../../lib/ipc";
  import { runtimeStore } from "../../lib/stores/runtime";
  import { settingsStore, toast, errText } from "../../lib/stores/ui";
  import ExitPrompt from "../../lib/components/ExitPrompt.svelte";
  import RuntimeButton from "../../lib/components/RuntimeButton.svelte";
  import iconSrc from "../../assets/brand/icon-src.webp";

  let { onOpenAdvanced, onToggleSidebar, brandWidth }: {
    onOpenAdvanced: () => void;
    onToggleSidebar: () => void;
    /** 品牌区宽度（与侧边栏对齐；侧边栏收起时只留开关）。 */
    brandWidth: string;
  } = $props();

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
  let maximized = $state(false);

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

  onMount(() => {
    if (!isTauri) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const win = getCurrentWindow();
    const sync = () => void win.isMaximized().then((v) => { if (!disposed) maximized = v; }).catch(() => {});
    sync();
    void win.onResized(sync).then((un) => { if (disposed) un(); else unlisten = un; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
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

  /** 非按钮区按下即整窗拖拽；双击切换最大化（按钮自身不触发）。 */
  function onDragMouseDown(e: MouseEvent) {
    if (!isTauri || e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return;
    if (e.detail === 2) void toggleMaximize();
    else void getCurrentWindow().startDragging();
  }

  async function minimize() {
    if (!isTauri) return;
    try { await getCurrentWindow().minimize(); }
    catch { /* 最小化失败静默（窗口可能已销毁） */ }
  }

  async function toggleMaximize() {
    if (!isTauri) return;
    try {
      await getCurrentWindow().toggleMaximize();
      maximized = await getCurrentWindow().isMaximized();
    } catch { /* 最大化失败静默 */ }
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

  const dotClass = $derived(
    !rt || rt.phase === "stopped"
      ? "bg-muted-foreground/50"
      : rt.phase === "running"
        ? "bg-primary shadow-[0_0_8px_var(--color-primary)]"
        : "bg-brand-pink animate-pulse",
  );
  const windowButton = "flex h-full w-11 items-center justify-center text-sidebar-foreground/70 transition-colors hover:bg-foreground/8 hover:text-foreground";
</script>

<!-- 标题栏整行可拖拽：非按钮区 mousedown 由 startDragging 接管（按钮已排除） -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="relative z-30 flex h-10 shrink-0 items-center bg-sidebar text-sidebar-foreground select-none"
  data-testid="titlebar"
  onmousedown={onDragMouseDown}
>
  <div class="flex shrink-0 items-center gap-1.5 self-stretch pl-2 transition-[width] duration-300" style:width={brandWidth}>
    <button type="button" class="flex size-7 shrink-0 items-center justify-center rounded-lg text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent hover:text-foreground"
      title="展开 / 收起侧边栏" aria-label="展开或收起侧边栏" data-testid="sidebar-toggle" onclick={onToggleSidebar}>
      <PanelLeft class="size-4" />
    </button>
    <img src={iconSrc} alt="" class="ml-1 size-5 shrink-0 rounded-md object-cover ring-1 ring-primary/50" />
    <p class="min-w-0 truncate text-[13px] font-semibold tracking-wide text-foreground">
      Kotone <span class="font-normal text-sidebar-foreground/70">琴音</span>
    </p>
  </div>

  <!-- 状态胶囊：状态灯 + 相位 + 快捷键状态 -->
  <div class="flex min-w-0 flex-1 items-center px-2">
    <div class="flex min-w-0 items-center gap-2 rounded-full px-2.5 py-1 text-xs" title={statusLine}>
      <span class="relative flex size-2 shrink-0">
        <span class="size-2 rounded-full transition-colors {dotClass}"></span>
        {#if rt?.restartNeeded}
          <span class="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-warning ring-1 ring-sidebar" title="配置已变更，需重启生效"></span>
        {/if}
      </span>
      <span class="shrink-0 font-medium {rt?.phase === 'running' ? 'text-primary' : rt && rt.phase !== 'stopped' ? 'text-brand-pink' : 'text-sidebar-foreground'}">{phaseLabel}</span>
      <span class="hidden min-w-0 truncate text-sidebar-foreground/60 md:inline">{statusLine}</span>
    </div>
  </div>

  <div class="flex h-full shrink-0 items-center">
    {#if rt?.phase === "running" && resourceText}
      <span class="mr-2 hidden text-[11px] text-sidebar-foreground/60 tabular-nums lg:inline" title="Kotone 进程资源占用">{resourceText}</span>
    {/if}
    <RuntimeButton variant="titlebar" {onOpenAdvanced} />
    <div class="mx-1 h-4 w-px bg-hairline"></div>
    <button class={windowButton} title="最小化" aria-label="最小化" onclick={() => void minimize()}><Minus class="size-4" /></button>
    <button class={windowButton} title={maximized ? "还原" : "最大化"} aria-label={maximized ? "还原" : "最大化"} data-testid="window-maximize" onclick={() => void toggleMaximize()}>
      {#if maximized}<Copy class="size-3.5 -scale-x-100" />{:else}<Square class="size-3.5" />{/if}
    </button>
    <button class="flex h-full w-11 items-center justify-center text-sidebar-foreground/70 transition-colors hover:bg-[#e81123] hover:text-white" title="关闭" aria-label="关闭" onclick={() => void close()}>
      <X class="size-4" />
    </button>
  </div>
</header>

{#if showExitPrompt}
  <ExitPrompt busy={exiting} onExit={() => void quitApp()} onMinimize={() => void minimizeToTray()} />
{/if}
