<script lang="ts">
  /*
   * 操作面板（main 窗 decorations:false，方向 B，高 150px）：
   * 替代细标题栏的宽面板——
   * - 左：Kotone 圆形大图标（青光晕）+ 名称品牌位；
   * - 中：状态区（运行状态大灯 + 相位文案 + 引擎 / 模型 / 交互模式 chips，
   *   restartNeeded 黄角标 +「需重启」徽章，runtimeStore 实时反映）；
   * - 右下角：启动/停止/重启生效 主 CTA（面板主行动点）+ 最小化 / 关闭；
   * - 面板非按钮区均为 data-tauri-drag-region 拖拽区。
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
  class="relative z-20 flex h-[150px] shrink-0 flex-col overflow-hidden border-b border-white/8 bg-kotone-panel/60 px-5 pt-4 pb-4"
>
  <!-- 面板背景光晕（方向 B 霓虹语言） -->
  <div
    class="pointer-events-none absolute -top-20 left-1/3 h-40 w-72 rounded-full bg-kotone-violet/20 blur-3xl"
  ></div>
  <div
    class="pointer-events-none absolute -bottom-16 right-1/4 h-32 w-56 rounded-full blur-3xl transition-colors duration-500 {rt?.phase ===
    'running'
      ? 'bg-kotone-cyan/15'
      : 'bg-kotone-pink/10'}"
  ></div>

  <div class="relative flex flex-1 items-center gap-5">
    <!-- 左：品牌位 -->
    <div data-tauri-drag-region class="flex shrink-0 items-center gap-3">
      <img
        src={iconSrc}
        alt=""
        class="h-14 w-14 rounded-full ring-2 ring-kotone-cyan/70 shadow-glow-cyan-lg object-cover"
      />
      <div data-tauri-drag-region>
        <p data-tauri-drag-region class="text-base font-bold tracking-wide">
          Kotone <span class="font-medium text-white/55">琴音</span>
        </p>
        <p data-tauri-drag-region class="mt-0.5 text-[10px] text-white/35">
          游戏语音输入 · 深夜直播间待命
        </p>
      </div>
    </div>

    <!-- 中：状态区 -->
    <div data-tauri-drag-region class="flex min-w-0 flex-1 flex-col items-center gap-2">
      <div data-tauri-drag-region class="flex items-center gap-2.5">
        <span class="relative flex h-3.5 w-3.5 shrink-0">
          <span
            class="h-3.5 w-3.5 rounded-full transition-colors {!rt || rt.phase === 'stopped'
              ? 'bg-white/25'
              : rt.phase === 'running'
                ? 'bg-kotone-cyan shadow-glow-cyan animate-pulse'
                : 'bg-kotone-pink shadow-glow-pink animate-ping'}"
          ></span>
          {#if rt?.restartNeeded}
            <span
              class="absolute -top-1 -right-1 h-2 w-2 rounded-full bg-yellow-400 ring-2 ring-kotone-panel"
              title="配置已变更，需重启生效"
            ></span>
          {/if}
        </span>
        <span data-tauri-drag-region class="text-sm font-semibold {rt?.phase === 'running'
          ? 'text-kotone-cyan'
          : rt && rt.phase !== 'stopped'
            ? 'text-kotone-pink'
            : 'text-white/55'}">
          {phaseLabel}
        </span>
        {#if rt?.restartNeeded}
          <span
            class="rounded bg-yellow-400/15 px-1.5 py-0.5 text-[10px] font-semibold text-yellow-300 ring-1 ring-yellow-400/40"
          >
            需重启
          </span>
        {/if}
      </div>
      {#if rt && rt.phase !== "stopped"}
        <div data-tauri-drag-region class="flex max-w-full items-center gap-1.5">
          <span
            class="truncate rounded-lg bg-kotone-cyan/10 px-2 py-1 text-[11px] text-kotone-cyan/90 ring-1 ring-kotone-cyan/25"
          >
            {rt.engineName ?? rt.engineId ?? "未知引擎"}
          </span>
          {#if rt.modelId}
            <span
              class="truncate rounded-lg bg-white/6 px-2 py-1 text-[11px] text-white/60 ring-1 ring-white/12"
            >
              {rt.modelId}
            </span>
          {/if}
          <span
            class="shrink-0 rounded-lg bg-kotone-violet/15 px-2 py-1 text-[11px] text-kotone-violet ring-1 ring-kotone-violet/30"
          >
            {rt.interactionMode ? (modeNames[rt.interactionMode] ?? rt.interactionMode) : "兼容模式"}
          </span>
        </div>
      {:else}
        <p data-tauri-drag-region class="text-[11px] text-white/35">
          热键未注册 · 点右下角「启动」开始说话
        </p>
      {/if}
    </div>
  </div>

  <!-- 右下角：主 CTA + 窗口控制 -->
  <div class="absolute right-4 bottom-3.5 flex items-center gap-2">
    <button
      class="flex h-9 items-center gap-2 rounded-[var(--radius-kotone-card)] px-5 text-sm font-bold transition active:scale-95 disabled:cursor-not-allowed {busy ||
      acting
        ? 'bg-white/10 text-white/50'
        : !rt || rt.phase === 'stopped'
          ? 'bg-kotone-pink text-white shadow-glow-pink-lg hover:brightness-110'
          : rt.restartNeeded
            ? 'bg-yellow-400/90 text-kotone-deep shadow-[0_0_18px_rgba(250,204,21,0.4)] hover:brightness-110'
            : 'bg-kotone-cyan/15 text-kotone-cyan ring-1 ring-kotone-cyan/50 hover:bg-kotone-cyan/25'}"
      disabled={busy || acting || !rt}
      onclick={() => void onMainButton()}
    >
      {#if busy || acting}
        <span class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-white/25 border-t-white/80"></span>
        {rt?.phase === "stopping" ? "停止中" : "启动中"}
      {:else if !rt || rt.phase === "stopped"}
        ▶ 启动
      {:else if rt.restartNeeded}
        重启生效
      {:else}
        ■ 停止
      {/if}
    </button>
    <button
      class="flex h-9 w-9 items-center justify-center rounded-[var(--radius-kotone-card)] text-white/55 ring-1 ring-white/10 transition hover:bg-white/10 hover:text-white"
      title="最小化"
      onclick={() => void minimize()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M5 12h14" stroke-linecap="round" /></svg>
    </button>
    <button
      class="flex h-9 w-9 items-center justify-center rounded-[var(--radius-kotone-card)] text-white/55 ring-1 ring-white/10 transition hover:bg-kotone-pink/80 hover:text-white"
      title="关闭（隐藏到托盘）"
      onclick={() => void close()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" /></svg>
    </button>
  </div>
</header>
