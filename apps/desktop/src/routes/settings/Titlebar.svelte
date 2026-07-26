<script lang="ts">
  /*
   * 自绘标题栏（main 窗 decorations:false，方向 B 风格，高 40px）：
   * - 左：Kotone 图标 + 名称；栏体为 data-tauri-drag-region 拖拽区；
   * - 中：运行状态灯（Running 青呼吸 / Starting·Stopping 品红脉冲 / Stopped 灰，
   *   restartNeeded 黄角标）+ 当前引擎 + 交互模式（runtimeStore 实时反映）；
   * - 右：启动/停止/重启生效 主按钮 + 最小化 / 关闭（关闭 = 隐藏到托盘）。
   */
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { isTauri, startRuntime, stopRuntime } from "../../lib/ipc";
  import { runtimeStore } from "../../lib/stores/runtime";
  import { toast, errText } from "../../lib/stores/ui";
  import iconSrc from "../../assets/brand/icon-src.png";

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

  const statusText = $derived(() => {
    if (!rt) return "状态未知";
    if (rt.phase === "stopped") return "已停止";
    if (rt.stage) return stageNames[rt.stage] ?? "处理中…";
    if (rt.phase === "running") {
      const engine = rt.engineName ?? rt.engineId ?? "未知引擎";
      const mode = rt.interactionMode ? (modeNames[rt.interactionMode] ?? rt.interactionMode) : "兼容模式";
      return rt.restartNeeded ? `${engine} · 变更待重启` : `${engine} · ${mode}`;
    }
    return "处理中…";
  });

  async function onMainButton() {
    if (!rt || busy || acting) return;
    acting = true;
    try {
      if (rt.phase === "running" && !rt.restartNeeded) {
        await stopRuntime();
      } else {
        // stopped → 启动；running + restartNeeded → 重启（stop+start，壳侧编排）
        await startRuntime();
      }
    } catch (e) {
      toast(false, errText(e));
    } finally {
      acting = false;
    }
  }

  async function minimize() {
    if (isTauri) await getCurrentWindow().minimize();
  }

  /** 关闭 = 隐藏到托盘（与 CloseRequested 拦截语义一致；退出走托盘菜单） */
  async function close() {
    if (isTauri) await getCurrentWindow().hide();
  }
</script>

<header
  data-tauri-drag-region
  class="relative z-20 flex h-10 shrink-0 items-center gap-3 border-b border-white/8 bg-kotone-deep/80 pr-2 pl-3"
>
  <!-- 左：图标 + 名称 -->
  <div data-tauri-drag-region class="flex items-center gap-2">
    <img
      src={iconSrc}
      alt=""
      class="h-6 w-6 rounded-full ring-1 ring-kotone-cyan/70 shadow-glow-cyan object-cover"
    />
    <span data-tauri-drag-region class="text-xs font-bold tracking-wide">
      Kotone <span class="font-medium text-white/55">琴音</span>
    </span>
  </div>

  <!-- 中：运行状态灯 + 引擎/模式 -->
  <div data-tauri-drag-region class="flex min-w-0 flex-1 items-center justify-center gap-2">
    <span class="relative flex h-2.5 w-2.5 shrink-0">
      <span
        class="h-2.5 w-2.5 rounded-full {!rt || rt.phase === 'stopped'
          ? 'bg-white/30'
          : rt.phase === 'running'
            ? 'bg-kotone-cyan shadow-glow-cyan animate-pulse'
            : 'bg-kotone-pink shadow-glow-pink animate-ping'}"
      ></span>
      {#if rt?.restartNeeded}
        <span
          class="absolute -top-1 -right-1 h-1.5 w-1.5 rounded-full bg-yellow-400 ring-1 ring-kotone-deep"
          title="配置已变更，需重启生效"
        ></span>
      {/if}
    </span>
    <span data-tauri-drag-region class="truncate text-[11px] text-white/60">{statusText()}</span>
    {#if rt?.restartNeeded}
      <span class="shrink-0 rounded bg-yellow-400/15 px-1.5 py-0.5 text-[10px] font-semibold text-yellow-300 ring-1 ring-yellow-400/40">
        需重启
      </span>
    {/if}
  </div>

  <!-- 右：启动/停止 + 窗口控制 -->
  <button
    class="flex h-6.5 shrink-0 items-center gap-1.5 rounded-lg px-3 text-[11px] font-semibold transition active:scale-95 disabled:cursor-not-allowed {busy || acting
      ? 'bg-white/10 text-white/50'
      : !rt || rt.phase === 'stopped'
        ? 'bg-kotone-pink text-white shadow-glow-pink hover:brightness-110'
        : rt.restartNeeded
          ? 'bg-yellow-400/90 text-kotone-deep hover:brightness-110'
          : 'bg-kotone-cyan/15 text-kotone-cyan ring-1 ring-kotone-cyan/50 hover:bg-kotone-cyan/25'}"
    disabled={busy || acting || !rt}
    onclick={() => void onMainButton()}
  >
    {#if busy || acting}
      <span
        class="h-3 w-3 animate-spin rounded-full border border-white/25 border-t-white/80"
      ></span>
      {rt?.phase === "stopping" ? "停止中" : "启动中"}
    {:else if !rt || rt.phase === "stopped"}
      启动
    {:else if rt.restartNeeded}
      重启生效
    {:else}
      停止
    {/if}
  </button>

  <button
    class="flex h-6.5 w-8 shrink-0 items-center justify-center rounded-lg text-white/60 transition hover:bg-white/10 hover:text-white"
    title="最小化"
    onclick={() => void minimize()}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-3.5 w-3.5"><path d="M5 12h14" stroke-linecap="round" /></svg>
  </button>
  <button
    class="flex h-6.5 w-8 shrink-0 items-center justify-center rounded-lg text-white/60 transition hover:bg-kotone-pink/80 hover:text-white"
    title="关闭（隐藏到托盘）"
    onclick={() => void close()}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-3.5 w-3.5"><path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" /></svg>
  </button>
</header>
