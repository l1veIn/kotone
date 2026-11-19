<script lang="ts">
  /*
   * 设置窗口的自绘标题栏：运行状态、资源占用和窗口控制。
   */
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { exit } from "@tauri-apps/plugin-process";
  import { getResourceUsage, isTauri } from "../../lib/ipc";
  import { runtimeStore, voiceInputStatus } from "../../lib/stores/runtime";
  import { toast, errText } from "../../lib/stores/ui";
  import ExitPrompt from "../../lib/components/ExitPrompt.svelte";
  import iconSrc from "../../assets/brand/icon-src.webp";

  let { showVoiceStatus }: { showVoiceStatus: boolean } = $props();

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
  class="relative z-20 flex h-11 shrink-0 items-center justify-between border-b border-white/8 bg-[#171729] select-none"
  onmousedown={onDragMouseDown}
>
  <div class="relative flex min-w-0 flex-1 items-center self-stretch">
    <div class="flex w-46 shrink-0 items-center gap-2.5 self-stretch border-r border-white/8 px-4 max-[720px]:w-14 max-[720px]:justify-center max-[720px]:px-0">
      <img
        src={iconSrc}
        alt=""
        class="size-6 shrink-0 rounded-[22%] object-cover"
      />
      <p class="shrink-0 text-xs font-bold tracking-wide max-[720px]:hidden">
        Kotone <span class="font-medium text-white/55">琴音</span>
      </p>
    </div>
    {#if showVoiceStatus}
      <div class="flex min-w-0 items-center gap-2 px-4 text-xs text-white/65">
        <span class="size-1.5 shrink-0 rounded-full {rt?.error ? 'bg-amber-300' : rt?.phase === 'running' ? 'bg-kotone-cyan' : 'bg-white/35'}" aria-hidden="true"></span>
        <span class="truncate" title={voiceInputStatus(rt)}>{voiceInputStatus(rt)}</span>
      </div>
    {/if}
  </div>

  <div class="relative flex h-full shrink-0 items-center">
    {#if rt?.phase === "running" && resourceText}
      <span class="mr-3 text-[10px] text-white/45 tabular-nums max-[720px]:hidden" title="Kotone 进程资源占用">
        {resourceText}
      </span>
    {/if}
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
