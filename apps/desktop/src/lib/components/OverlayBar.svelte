<script lang="ts">
  /*
   * 悬浮录音条（方向 B「中继站」：直播间弹幕气泡风格）。
   * 状态驱动 UI：orchestrator 是唯一状态所有者，这里只渲染 appState。
   *
   *   listening    麦克风呼吸光晕 + 流式引擎 partial 上屏 / 非流式引擎青→品红渐变声波
   *   transcribing 「转写中…」
   *   preview      聊天气泡（青色描边 + 小尾巴）+「再按一次发送 · Esc 重说」
   *   sending      品红光晕脉冲
   *   success      Kotone 贴纸弹出 + 气泡「收到，已发送！✨」
   *   error        amazed 贴纸 + 错误摘要 + 重试 / 关闭
   */
  import { fade, fly, scale } from "svelte/transition";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState } from "../stores/state";
  import {
    confirmSend,
    cancelSession,
    simulateSend,
    getSettings,
    isTauri,
    saveOverlayPosition,
    type OverlayConfig,
  } from "../ipc";
  import Waveform from "./Waveform.svelte";
  import stickerProud from "../../assets/brand/stickers/proud.webp";
  import stickerAmazed from "../../assets/brand/stickers/amazed.webp";

  const stateLabel: Record<string, string> = {
    idle: "Kotone 待机",
    listening: "语音输入中…",
    transcribing: "转写中",
    preview: "预览确认",
    sending: "发送中",
    success: "已发送",
    error: "出错了",
  };

  /** 操作中的瞬时错误提示（如 confirm 调用失败） */
  let actionError = $state("");
  /** 重试后的本地提示 */
  let actionHint = $state("");
  /** preview 提示中的热键名（动态读配置，读取失败回退 CapsLock） */
  let hotkeyLabel = $state("CapsLock");
  /** overlay.style：true = 胶囊布局（窗口几何由后端 SetWindowPos 居中靠下重排） */
  let capsule = $state(false);
  let draggable = $state(true);
  let clickThrough = $state(false);
  let manualDragArmed = false;
  let manualDragMoved = false;
  let dragSettleTimer: ReturnType<typeof setTimeout> | undefined;
  let dragDisarmTimer: ReturnType<typeof setTimeout> | undefined;

  function applyOverlayConfig(config: OverlayConfig) {
    capsule = config.style === "capsule";
    draggable = config.draggable;
    clickThrough = config.clickThrough;
  }

  onMount(async () => {
    try {
      const s = await getSettings();
      hotkeyLabel = s.hotkey.key;
      applyOverlayConfig(s.overlay);
    } catch {
      /* 读取失败保留默认值 */
    }
  });

  // 设置页切换 overlay 配置 → 后端应用几何/点击穿透，并广播给前端更新交互。
  onMount(() => {
    if (!isTauri) return;
    let un: (() => void) | undefined;
    void (async () => {
      un = await listen<OverlayConfig>("kotone://overlay-config", (ev) => {
        applyOverlayConfig(ev.payload);
      });
    })();
    return () => un?.();
  });

  // 预设位置也会产生 moved 事件，所以只在用户从悬浮窗发起拖动后记录坐标。
  onMount(() => {
    if (!isTauri) return;
    let unMoved: (() => void) | undefined;
    void (async () => {
      unMoved = await getCurrentWindow().onMoved(() => {
        if (!manualDragArmed) return;
        manualDragMoved = true;
        if (dragSettleTimer) clearTimeout(dragSettleTimer);
        dragSettleTimer = setTimeout(() => {
          manualDragArmed = false;
          void saveOverlayPosition();
        }, 300);
      });
    })();
    return () => {
      unMoved?.();
      if (dragSettleTimer) clearTimeout(dragSettleTimer);
      if (dragDisarmTimer) clearTimeout(dragDisarmTimer);
    };
  });

  function onDragPointerDown(event: PointerEvent) {
    if (!isTauri || !draggable || clickThrough || event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    manualDragArmed = true;
    manualDragMoved = false;
    if (dragSettleTimer) clearTimeout(dragSettleTimer);
    if (dragDisarmTimer) clearTimeout(dragDisarmTimer);
    // 单击但没有移动时及时撤销，避免后续预设移动被误判成人工拖动。
    dragDisarmTimer = setTimeout(() => {
      if (!manualDragMoved) manualDragArmed = false;
    }, 3000);
    // data-tauri-drag-region 只在 event.target 自身带属性时生效（子元素无法拖动），
    // 改为与 Titlebar 一致的 startDragging()：任意非交互位置按下即可整窗拖动。
    void getCurrentWindow().startDragging();
  }

  let textScrollEl: HTMLDivElement | undefined = $state();
  let previewScrollEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    if ($appState.state === "preview") {
      actionError = "";
      actionHint = "";
    }
  });

  // partial / 预览文本更新时滚动到底部（流式上屏、card 预览气泡）
  $effect(() => {
    void $appState.partialText;
    void $appState.finalText;
    if (textScrollEl) textScrollEl.scrollTop = textScrollEl.scrollHeight;
    if (previewScrollEl) previewScrollEl.scrollTop = previewScrollEl.scrollHeight;
  });

  async function onConfirm() {
    actionError = "";
    try {
      await confirmSend();
    } catch (e) {
      actionError = String(e);
    }
  }

  async function onCancel() {
    actionError = "";
    try {
      await cancelSession();
    } catch (e) {
      actionError = String(e);
    }
  }

  /** 错误重试：发送中失败的文本走 simulate_send 重发，然后回待机 */
  async function onRetry() {
    actionError = "";
    actionHint = "";
    const text = $appState.errorText ?? $appState.finalText;
    if (!text) {
      await onCancel();
      return;
    }
    try {
      await simulateSend(text);
      actionHint = "已重试发送";
      await cancelSession();
    } catch (e) {
      actionError = String(e);
    }
  }

  const displayError = $derived(actionError || $appState.errorMessage || "未知错误");
</script>

<!--
  两种样式（overlay.style）：
  - card：480×120 窗口内铺满一条圆角悬浮条；非按钮区 pointerdown 由 startDragging() 整窗拖动。
    整体小巧不遮挡游戏：透明背景 + 微弱青光晕描边面板。
  - capsule：520×64 窗口（后端 SetWindowPos 水平居中靠下，底部留 48px），
    胶囊本体宽度随内容伸缩（fit-content），轻装饰——Win11 语音输入条风格。
  单行文本用 .ellipsis-head（direction:rtl）让省略号落在行首：显示最新尾部、头部滚出，
  文本末尾补 LRM（&#x200E;）防中英混排时尾部标点被 bidi 算法翻转。
-->
{#if capsule}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="flex h-full items-center justify-center select-none {draggable && !clickThrough
      ? 'cursor-move'
      : ''}"
    onpointerdown={onDragPointerDown}
  >
    <div
      class="flex max-w-full items-center gap-2.5 rounded-full bg-kotone-deep/96 py-2 pr-4 pl-3.5 ring-1 ring-kotone-cyan/35"
      style:width="fit-content"
    >
      {#if $appState.state === "listening"}
        <!-- 收到真实 partial 后立即上屏；尚无 partial（含非流式引擎）时显示声波。 -->
        <span class="mic-breath h-2.5 w-2.5 shrink-0 rounded-full bg-kotone-cyan"></span>
        {#if $appState.partialText}
          <p
            class="ellipsis-head max-w-[340px] truncate text-sm text-white"
            in:fade={{ duration: 150 }}
          >
            {$appState.partialText}&#x200E;
          </p>
        {:else}
          <Waveform level={$appState.level} bars={12} />
        {/if}
      {:else if $appState.state === "transcribing"}
        <span class="spinner inline-block h-3.5 w-3.5 shrink-0 rounded-full"></span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-kotone-violet"
          in:fade={{ duration: 150 }}
        >
          {$appState.partialText || "转写中…"}&#x200E;
        </p>
      {:else if $appState.state === "preview"}
        <p
          class="ellipsis-head max-w-[260px] truncate text-sm text-white"
          in:fade={{ duration: 150 }}
        >
          {$appState.finalText}&#x200E;
        </p>
        <span class="shrink-0 text-[10px] whitespace-nowrap text-white/40">
          {hotkeyLabel} 发送 · Esc 重说
        </span>
        <button
          class="shrink-0 rounded-full bg-kotone-pink px-2.5 py-0.5 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95"
          onclick={() => void onConfirm()}
        >
          发送
        </button>
      {:else if $appState.state === "sending"}
        <span class="send-pulse inline-block h-2.5 w-2.5 shrink-0 rounded-full bg-kotone-pink"></span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-white/70"
          in:fade={{ duration: 150 }}
        >
          {$appState.finalText || "发送中"}&#x200E;
        </p>
      {:else if $appState.state === "success"}
        <!-- 发送后短暂显示结果（随后按 overlay.visibility 规则隐藏/驻留） -->
        <span class="shrink-0 text-sm font-bold text-kotone-cyan">✓</span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-white"
          in:fade={{ duration: 150 }}
        >
          已发送 · {$appState.finalText}&#x200E;
        </p>
      {:else if $appState.state === "error"}
        <span class="shrink-0 text-sm font-bold text-kotone-pink">✗</span>
        <p class="max-w-[220px] truncate text-sm text-kotone-pink" title={displayError}>
          {displayError}
        </p>
        {#if $appState.errorText}
          <button
            class="shrink-0 rounded-full bg-kotone-pink/80 px-2.5 py-0.5 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95"
            onclick={() => void onRetry()}
          >
            重试
          </button>
        {/if}
        <button
          class="shrink-0 rounded-full bg-white/10 px-2.5 py-0.5 text-[11px] text-white/75 transition hover:bg-white/20 active:scale-95"
          onclick={() => void onCancel()}
        >
          关闭
        </button>
      {:else}
        <!-- idle（托盘手动唤起时可见） -->
        <span class="h-2 w-2 shrink-0 rounded-full bg-kotone-cyan/60"></span>
        <p class="shrink-0 text-sm whitespace-nowrap text-white/70">Kotone 待机 · 按热键说话</p>
      {/if}
    </div>
  </div>
{:else}
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="flex h-full items-stretch select-none {draggable && !clickThrough ? 'cursor-move' : ''}"
  onpointerdown={onDragPointerDown}
>
  <div
    class="flex w-full items-center gap-3 rounded-2xl bg-kotone-deep/96 px-4 py-2 ring-1 ring-inset ring-kotone-cyan/40"
  >
    {#if $appState.state === "listening"}
      <!-- 收到真实 partial 后立即上屏；尚无 partial（含非流式引擎）时显示声波。 -->
      <span class="mic-breath flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-kotone-cyan/12" in:fade={{ duration: 150 }}>
        <svg viewBox="0 0 24 24" fill="none" stroke="#00e5ff" stroke-width="2" class="h-4.5 w-4.5">
          <rect x="9" y="2" width="6" height="12" rx="3" />
          <path d="M5 10a7 7 0 0 0 14 0M12 19v3" stroke-linecap="round" />
        </svg>
      </span>
      {#if $appState.partialText}
        <div class="min-w-0 flex-1">
          <p class="text-[11px] leading-tight text-kotone-cyan/80">{stateLabel.listening}</p>
          <div bind:this={textScrollEl} class="kotone-scroll mt-0.5 max-h-14 overflow-y-auto pr-1">
            {#key $appState.partialText}
              <p class="text-sm leading-snug break-all text-white" in:fade={{ duration: 180 }}>
                {$appState.partialText}
              </p>
            {/key}
          </div>
        </div>
      {:else}
        <!-- 第一条 partial 到达前保持声波；非流式引擎会始终停留在这里。 -->
        <div class="flex min-w-0 flex-1 items-center justify-center" in:fade={{ duration: 150 }}>
          <Waveform level={$appState.level} />
        </div>
      {/if}
    {:else if $appState.state === "transcribing"}
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="flex items-center gap-2 text-sm text-kotone-violet">
          <span class="spinner inline-block h-3.5 w-3.5 rounded-full"></span>
          转写中…
        </p>
        {#if $appState.partialText}
          <p class="mt-0.5 truncate text-xs text-white/50">{$appState.partialText}</p>
        {/if}
      </div>
    {:else if $appState.state === "preview"}
      <!-- 预览（ADR-006 只读）：聊天气泡 + 热键确认/重说。悬浮条不抢焦点，主交互是热键 -->
      <div class="flex min-w-0 flex-1 flex-col gap-1.5" in:fade={{ duration: 150 }}>
        <div bind:this={previewScrollEl} class="bubble kotone-scroll max-h-14 overflow-y-auto px-3 py-1.5">
          <p class="text-sm leading-snug break-all text-white">{$appState.finalText}</p>
        </div>
        <p class="text-[11px] leading-tight text-white/45">
          再按 {hotkeyLabel} 发送 · Esc 重说
        </p>
        {#if actionError}
          <p class="truncate text-[11px] text-kotone-pink">{actionError}</p>
        {/if}
      </div>
      <div class="flex shrink-0 flex-col gap-1.5">
        <button
          class="rounded-lg bg-kotone-pink px-3 py-1 text-xs font-semibold text-white shadow-glow-pink transition hover:brightness-110 active:scale-95"
          onclick={() => void onConfirm()}
        >
          发送 ⏎
        </button>
        <button
          class="rounded-lg bg-white/10 px-3 py-1 text-xs text-white/75 transition hover:bg-white/20 active:scale-95"
          onclick={() => void onCancel()}
        >
          取消 Esc
        </button>
      </div>
    {:else if $appState.state === "sending"}
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="flex items-center gap-2 text-sm font-semibold text-kotone-pink">
          <span class="send-pulse inline-block h-3 w-3 rounded-full bg-kotone-pink"></span>
          发送中
        </p>
        <p class="mt-0.5 truncate text-xs text-white/50">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "success"}
      <!-- 已发送：Kotone 贴纸弹出 + 气泡 -->
      <img
        src={stickerProud}
        alt="Kotone 点赞"
        class="h-14 w-14 shrink-0 object-contain"
        in:scale={{ duration: 220, start: 0.5 }}
      />
      <div class="min-w-0 flex-1" in:fly={{ y: 6, duration: 200 }}>
        <div class="bubble inline-block max-w-full px-3 py-1.5">
          <p class="text-sm font-semibold text-white">收到，已发送！✨</p>
        </div>
        <p class="mt-1 truncate text-xs text-white/55">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "error"}
      <!-- 出错：amazed 贴纸 + 错误摘要 -->
      <img
        src={stickerAmazed}
        alt="Kotone 惊讶"
        class="h-12 w-12 shrink-0 object-contain"
        in:scale={{ duration: 200, start: 0.6 }}
      />
      <div class="min-w-0 flex-1" in:fly={{ y: 6, duration: 200 }}>
        <p class="line-clamp-2 text-[13px] leading-snug break-all font-medium text-kotone-pink" title={displayError}>
          {displayError}
        </p>
        {#if actionHint}
          <p class="mt-0.5 text-[11px] text-kotone-cyan">{actionHint}</p>
        {:else if $appState.errorText}
          <p class="mt-0.5 truncate text-[11px] text-white/50">保留文本：{$appState.errorText}</p>
        {/if}
      </div>
      <div class="flex shrink-0 gap-1.5">
        {#if $appState.errorText}
          <button
            class="rounded-lg bg-kotone-pink/80 px-2.5 py-1 text-xs font-semibold text-white transition hover:brightness-110 active:scale-95"
            onclick={() => void onRetry()}
          >
            重试
          </button>
        {/if}
        <button
          class="rounded-lg bg-white/10 px-2.5 py-1 text-xs text-white/75 transition hover:bg-white/20 active:scale-95"
          onclick={() => void onCancel()}
        >
          关闭
        </button>
      </div>
    {:else}
      <!-- idle（托盘手动唤起悬浮条时可见；正常流程会自动隐藏窗口） -->
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="text-sm font-medium text-white/80">Kotone 琴音</p>
        <p class="text-[11px] text-white/40">待机中 · 按热键开始说话</p>
      </div>
    {/if}
  </div>
</div>
{/if}

<style>
  /*
   * 头部省略号（capsule 单行文本）：direction:rtl 让 truncate 的省略号落在行首，
   * 显示最新尾部、头部滚出；text-align:left 保持视觉左对齐。
   * 文本尾部需补 LRM（\u200E）：中英混排时尾部标点/数字属中性字符，
   * 在 RTL 段落方向下会被 bidi 算法翻到行首，LRM 把它们锚回 LTR。
   * （中文/英文均为强 L 字符，语序本身不受 rtl 影响。）
   */
  .ellipsis-head {
    direction: rtl;
    text-align: left;
  }

  /* 聊天气泡：青色描边 + 左下小尾巴 */
  .bubble {
    position: relative;
    background: rgba(0, 229, 255, 0.08);
    border: 1px solid rgba(0, 229, 255, 0.45);
    border-radius: 12px;
  }
  .bubble::after {
    content: "";
    position: absolute;
    left: 14px;
    bottom: -5px;
    width: 8px;
    height: 8px;
    background: inherit;
    border-right: 1px solid rgba(0, 229, 255, 0.45);
    border-bottom: 1px solid rgba(0, 229, 255, 0.45);
    transform: rotate(45deg);
  }

  /* 麦克风呼吸光晕 */
  .mic-breath {
    animation: kotone-mic-breath 1.8s ease-in-out infinite;
  }
  @keyframes kotone-mic-breath {
    0%,
    100% {
      box-shadow: 0 0 6px rgba(0, 229, 255, 0.25);
    }
    50% {
      box-shadow: 0 0 18px rgba(0, 229, 255, 0.6);
    }
  }

  /* 发送中：品红光晕脉冲 */
  .send-pulse {
    animation: kotone-send-pulse 0.9s ease-in-out infinite;
  }
  @keyframes kotone-send-pulse {
    0%,
    100% {
      box-shadow: 0 0 4px rgba(255, 45, 120, 0.4);
      transform: scale(1);
    }
    50% {
      box-shadow: 0 0 16px rgba(255, 45, 120, 0.85);
      transform: scale(1.15);
    }
  }

  /* 转写中的旋转圈 */
  .spinner {
    border: 2px solid rgba(123, 47, 255, 0.25);
    border-top-color: #7b2fff;
    animation: kotone-spin 0.8s linear infinite;
  }
  @keyframes kotone-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* 细滚动条 */
  .kotone-scroll::-webkit-scrollbar {
    width: 4px;
  }
  .kotone-scroll::-webkit-scrollbar-thumb {
    background: rgba(0, 229, 255, 0.3);
    border-radius: 9999px;
  }
</style>
