<script lang="ts">
  /*
   * 自绘标题栏（main 窗 decorations:false，高 40px，与侧边栏同底色，参照 DeepSeek Harness 的
   * Windows 标题行）：
   * - 左：侧边栏开关 + Kotone 图标与名称；只有用本地语音识别时才有状态胶囊
   *   （未启动 / 准备中 / 已启动 / 出问题了，点开是 启动 / 停止 / 去语音输入设置，ADR 028）；
   *   在线识别打开就能用，这里什么都不显示；
   * - 右：运行中的 CPU / 内存（2s 轮询）+ 最小化 / 最大化（还原）/ 关闭；
   * - 非按钮区按下拖动窗口，双击切换最大化；
   * - 关闭 = 弹确认框：「退出」彻底退出进程，「最小化到托盘」保持隐藏语义。
   */
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { exit } from "@tauri-apps/plugin-process";
  import { ChevronDown, Copy, Minus, PanelLeft, Square, X } from "@lucide/svelte";
  import * as Menu from "$lib/components/ui/dropdown-menu";
  import { getResourceUsage, isTauri, startRuntime, stopRuntime } from "../../lib/ipc";
  import { LOCAL_ASR_LABELS } from "../../lib/ipc/contracts";
  import { asrStore, runtimeStore } from "../../lib/stores/runtime";
  import { toast, errText } from "../../lib/stores/ui";
  import ExitPrompt from "../../lib/components/ExitPrompt.svelte";
  import iconSrc from "../../assets/brand/icon-src.webp";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenVoiceInput, onToggleSidebar, brandWidth }: {
    /** 状态胶囊菜单里的「去语音输入设置」。 */
    onOpenVoiceInput: () => void;
    onToggleSidebar: () => void;
    /** 品牌区宽度（与侧边栏对齐；侧边栏收起时只留开关）。 */
    brandWidth: string;
  } = $props();

  const rt = $derived($runtimeStore);
  let showExitPrompt = $state(false);
  let exiting = $state(false);
  let maximized = $state(false);

  /** 运行中的资源占用文本（空 = 非 running 或尚未采样成功） */
  let resourceText = $state("");

  async function pollResourceUsage() {
    try {
      const u = await getResourceUsage();
      resourceText = m.shell_titlebar_resource_usage({ cpu: u.cpuPercent.toFixed(1), memory: Math.round(u.memoryBytes / 1024 / 1024) });
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

  const asr = $derived($asrStore);
  const localState = $derived(asr.startsWith("local-") ? (asr as keyof typeof LOCAL_ASR_LABELS) : null);
  let acting = $state(false);
  async function runAsr(action: "start" | "stop") {
    if (acting) return;
    acting = true;
    try {
      await (action === "start" ? startRuntime() : stopRuntime());
      toast(true, action === "start" ? m.shell_titlebar_asr_started() : m.shell_titlebar_asr_stopped());
    } catch (e) {
      toast(false, errText(e));
    } finally {
      acting = false;
    }
  }

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
      toast(false, m.shell_titlebar_minimize_to_tray_failed({ error: errText(e) }));
    }
  }

  async function quitApp() {
    if (exiting) return;
    exiting = true;
    try {
      await exit(0);
    } catch (e) {
      exiting = false;
      toast(false, m.shell_titlebar_quit_failed({ error: errText(e) }));
    }
  }

  const dotClass = $derived(
    localState === "local-running"
      ? "bg-primary shadow-[0_0_8px_var(--color-primary)]"
      : localState === "local-starting"
        ? "bg-warning animate-pulse"
        : localState === "local-error"
          ? "bg-destructive"
          : "bg-muted-foreground/50",
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
      title={m.shell_titlebar_toggle_sidebar_title()} aria-label={m.shell_titlebar_toggle_sidebar_label()} data-testid="sidebar-toggle" onclick={onToggleSidebar}>
      <PanelLeft class="size-4" />
    </button>
    <img src={iconSrc} alt="" class="ml-1 size-5 shrink-0 rounded-md object-cover ring-1 ring-primary/50" />
    <p class="min-w-0 truncate text-[13px] font-semibold tracking-wide text-foreground">
      Kotone <span class="font-normal text-sidebar-foreground/70">{m.shell_titlebar_brand_name()}</span>
    </p>
  </div>

  <!-- 本地语音识别的全局状态（只在用本地识别时出现，ADR 028 W7 / W9） -->
  <div class="flex min-w-0 flex-1 items-center px-2">
    {#if localState}
      <Menu.DropdownMenu>
        <Menu.DropdownMenuTrigger data-testid="titlebar-asr" data-asr={localState}
          class="flex min-w-0 items-center gap-2 rounded-full px-2.5 py-1 text-xs transition-colors hover:bg-sidebar-accent">
          <span class="size-2 shrink-0 rounded-full transition-colors {dotClass}"></span>
          <span class="truncate font-medium {localState === 'local-running' ? 'text-primary' : localState === 'local-error' ? 'text-destructive' : 'text-sidebar-foreground'}">{m.shell_titlebar_local_asr({ status: LOCAL_ASR_LABELS[localState] })}{localState === "local-starting" ? "…" : ""}</span>
          <ChevronDown class="size-3 shrink-0 opacity-60" />
        </Menu.DropdownMenuTrigger>
        <Menu.DropdownMenuContent align="start" class="w-52">
          {#if localState === "local-running"}
            <Menu.DropdownMenuItem data-testid="titlebar-asr-stop" disabled={acting} onSelect={() => void runAsr("stop")}>{m.shell_titlebar_asr_stop()}</Menu.DropdownMenuItem>
            {#if rt?.restartNeeded}
              <Menu.DropdownMenuItem data-testid="titlebar-asr-restart" disabled={acting} onSelect={() => void runAsr("start")}>{m.shell_titlebar_asr_restart()}</Menu.DropdownMenuItem>
            {/if}
          {:else}
            <Menu.DropdownMenuItem data-testid="titlebar-asr-start" disabled={acting || localState === "local-starting"} onSelect={() => void runAsr("start")}>
              {localState === "local-error" ? m.shell_titlebar_asr_retry() : m.shell_titlebar_asr_start()}
            </Menu.DropdownMenuItem>
          {/if}
          {#if localState === "local-error" && rt?.error}
            <p class="m-0 px-2 py-1.5 text-[11px] leading-relaxed text-destructive">{rt.error}</p>
          {/if}
          <Menu.DropdownMenuSeparator />
          <Menu.DropdownMenuItem data-testid="titlebar-asr-settings" onSelect={onOpenVoiceInput}>{m.shell_titlebar_voice_settings()}</Menu.DropdownMenuItem>
        </Menu.DropdownMenuContent>
      </Menu.DropdownMenu>
    {/if}
  </div>

  <div class="flex h-full shrink-0 items-center">
    {#if rt?.phase === "running" && resourceText}
      <span class="mr-2 hidden text-[11px] text-sidebar-foreground/60 tabular-nums lg:inline" title={m.shell_titlebar_resource_title()}>{resourceText}</span>
    {/if}
    <div class="mx-1 h-4 w-px bg-hairline"></div>
    <button class={windowButton} title={m.shell_titlebar_minimize()} aria-label={m.shell_titlebar_minimize()} onclick={() => void minimize()}><Minus class="size-4" /></button>
    <button class={windowButton} title={maximized ? m.shell_titlebar_restore() : m.shell_titlebar_maximize()} aria-label={maximized ? m.shell_titlebar_restore() : m.shell_titlebar_maximize()} data-testid="window-maximize" onclick={() => void toggleMaximize()}>
      {#if maximized}<Copy class="size-3.5 -scale-x-100" />{:else}<Square class="size-3.5" />{/if}
    </button>
    <button class="flex h-full w-11 items-center justify-center text-sidebar-foreground/70 transition-colors hover:bg-[#e81123] hover:text-white" title={m.shell_titlebar_close()} aria-label={m.shell_titlebar_close()} onclick={() => void close()}>
      <X class="size-4" />
    </button>
  </div>
</header>

{#if showExitPrompt}
  <ExitPrompt busy={exiting} onExit={() => void quitApp()} onMinimize={() => void minimizeToTray()} />
{/if}
