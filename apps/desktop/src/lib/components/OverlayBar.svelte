<script lang="ts">
  /*
   * 悬浮录音条（方向 B「中继站」：直播间弹幕气泡风格）。
   * 状态驱动 UI：orchestrator 是唯一状态所有者，这里只渲染 appState。
   *
   *   listening    麦克风呼吸光晕 + 流式引擎 partial 上屏 / 非流式引擎青→品红渐变声波
   *   transcribing 「转写中…」
   *   processing   「后处理中…」+ 识别原文
   *   preview      聊天气泡（青色描边 + 小尾巴）+「再按一次发送 · Esc 重说」
   *   sending      品红光晕脉冲
   *   success      Kotone 贴纸弹出 + 气泡「收到，已发送！✨」
   *   error        amazed 贴纸 + 错误摘要 + 重试 / 关闭
   */
  import { hotkeyLabel as formatHotkey } from "../hotkeyCombo";
  import { fade, fly, scale } from "svelte/transition";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appState } from "../stores/state";
  import {
    confirmSend,
    cancelSession,
    getSettings,
    isTauri,
    dragOverlay,
    overlayGiveBack,
    overlayPointerEnter,
    saveOverlayPosition,
    type OverlayConfig,
  } from "../ipc";
  import Waveform from "./Waveform.svelte";
  import stickerProud from "../../assets/brand/stickers/proud.webp";
  import stickerAmazed from "../../assets/brand/stickers/amazed.webp";
  import { m } from "$lib/paraglide/messages.js";

  const stateLabel: Record<string, string> = {
    idle: m.voice_overlay_state_idle(),
    listening: m.voice_overlay_state_listening(),
    transcribing: m.voice_overlay_state_transcribing(),
    processing: m.voice_overlay_state_processing(),
    preview: m.voice_overlay_state_preview(),
    sending: m.voice_overlay_state_sending(),
    success: m.voice_overlay_state_success(),
    error: m.voice_overlay_state_error(),
  };

  /** 操作中的瞬时错误提示（如 confirm 调用失败） */
  let actionError = $state("");
  /** 重试后的本地提示 */
  let actionHint = $state("");
  /** IPC 往返期间立即锁住发送入口；后端状态事件到达前也不能重复提交。 */
  let sendPending = $state(false);
  /** preview 提示中的热键名（动态读配置，读取失败回退 CapsLock） */
  let hotkeyLabel = $state("CapsLock");
  let callHotkeyLabel: string = $state(m.voice_hotkey_name_call());
  const assistantHotkeyLabel = $derived(callHotkeyLabel);
  const assistantListeningHint = $derived(m.voice_overlay_assistant_listening({ hotkey: assistantHotkeyLabel }));
  const confirmLabel = $derived($appState.purpose === "assistant" ? m.voice_overlay_confirm_assistant() : m.voice_overlay_confirm_send());
  const confirmHotkey = $derived($appState.purpose === "assistant" ? assistantHotkeyLabel : hotkeyLabel);
  const previewHint = $derived(m.voice_overlay_preview_hint({ hotkey: confirmHotkey, action: confirmLabel }));
  /** overlay.style：true = 胶囊布局（窗口几何由后端 SetWindowPos 居中靠下重排） */
  let capsule = $state(false);
  let draggable = $state(true);
  let clickThrough = $state(false);
  let manualDragArmed = false;
  let manualDragMoved = false;
  let dragSettleTimer: ReturnType<typeof setTimeout> | undefined;
  let dragDisarmTimer: ReturnType<typeof setTimeout> | undefined;
  // 原生隐藏窗口不一定触发 visibilitychange，两个来源都要允许才播放动效。
  let documentVisible = $state(false);
  let nativeVisible = $state(!isTauri);
  const renderActive = $derived(documentVisible && nativeVisible);
  /** 语音识别没准备好时按了语音输入热键：后端发来的提示（ADR 028，D8）；null = 收起。 */
  let notReadyHint = $state<{ title: string; detail: string } | null>(null);
  // 一开始录音（任何非 idle 状态）就撤掉提示
  $effect(() => {
    if ($appState.state !== "idle") notReadyHint = null;
  });

  onMount(() => {
    if (!isTauri) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ title: string; detail: string } | null>("kotone://overlay-hint", ({ payload }) => {
      if (!disposed) notReadyHint = payload;
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  onMount(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const updateDocumentVisibility = () => {
      documentVisible = document.visibilityState !== "hidden";
    };
    updateDocumentVisibility();
    document.addEventListener("visibilitychange", updateDocumentVisibility);
    if (isTauri) {
      void (async () => {
        const cleanup = await listen<boolean>("kotone://overlay-visibility", ({ payload }) => {
          if (disposed) return;
          revision++;
          nativeVisible = payload;
        });
        if (disposed) { cleanup(); return; }
        unlisten = cleanup;
        const snapshotRevision = revision;
        const visible = await getCurrentWindow().isVisible();
        if (!disposed && revision === snapshotRevision) nativeVisible = visible;
      })().catch(() => { /* 可见性未知时保留静态 UI，避免隐藏窗口持续渲染。 */ });
    }
    return () => {
      disposed = true;
      unlisten?.();
      document.removeEventListener("visibilitychange", updateDocumentVisibility);
    };
  });

  function applyOverlayConfig(config: OverlayConfig) {
    capsule = config.style === "capsule";
    draggable = config.draggable;
    clickThrough = config.clickThrough;
  }

  onMount(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    void (async () => {
      if (isTauri) {
        const cleanup = await listen<{hotkey: string; assistantHotkey: string}>("kotone://input-config", ({payload}) => {
          if (disposed) return;
          revision++;
          hotkeyLabel = payload.hotkey;
          callHotkeyLabel = payload.assistantHotkey ? formatHotkey(payload.assistantHotkey) : m.voice_hotkey_name_call();
        });
        if (disposed) { cleanup(); return; }
        unlisten = cleanup;
      }
      const snapshotRevision = revision;
      const s = await getSettings();
      if (disposed) return;
      if (revision === snapshotRevision) {
        hotkeyLabel = s.hotkey.key;
        const callHotkey = s.assistant.hotkey;
        callHotkeyLabel = callHotkey ? formatHotkey(callHotkey) : m.voice_hotkey_name_call();
      }
      applyOverlayConfig(s.overlay);
    })().catch(() => { /* Keep useful defaults if the initial snapshot fails. */ });
    return () => { disposed = true; unlisten?.(); };
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

  // 点悬浮窗会把它激活（不可聚焦的窗口里 WebView2 收不到点击，所以它只能可聚焦）：鼠标移上来时记下玩家在用的窗口，
  // 点完把前台还给它，游戏马上又收得到键（共享资源评估第二节第 5 条）。拖动松开时后端自己还。
  function onPointerEnter() {
    if (isTauri && !clickThrough) void overlayPointerEnter();
  }

  function onPointerUp(event: PointerEvent) {
    if (!isTauri || clickThrough || event.button !== 0) return;
    // 等这次点击的处理先跑（按钮的 onclick 在 pointerup 之后）
    setTimeout(() => void overlayGiveBack(), 0);
  }

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
    // 任意非交互位置按下即可整窗拖动：后端跟着光标挪，松开时把前台还给游戏（drag_overlay；系统的
    // startDragging() 是个模态循环，结束时页面不知道，没法及时还）。
    void dragOverlay();
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
    if (sendPending) return;
    sendPending = true;
    actionError = "";
    try {
      await confirmSend();
    } catch (e) {
      actionError = String(e);
    } finally {
      sendPending = false;
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

  /** 错误重试：沿用 orchestrator 保留的文本、profile、频道与目标窗口。 */
  async function onRetry() {
    if (sendPending) return;
    actionError = "";
    actionHint = "";
    const text = $appState.errorText ?? $appState.finalText;
    if (!text) {
      await onCancel();
      return;
    }
    sendPending = true;
    try {
      await confirmSend();
      actionHint = $appState.purpose === "assistant" ? m.voice_overlay_retried_assistant() : m.voice_overlay_retried_send();
    } catch (e) {
      actionError = String(e);
    } finally {
      sendPending = false;
    }
  }

  const displayError = $derived(actionError || $appState.errorMessage || m.voice_error_unknown());
  /** 鼠标模式：穿透（默认）时 overlay 窗口收不到任何鼠标事件，鼠标按钮只会误导用户。 */
  const mouseInteractive = $derived(!clickThrough);
  /** Error 态热键语义：保留文本 = 重试发送；无文本（如空转录）= 重说。 */
  const errorAction = $derived($appState.errorText ? ($appState.purpose === "assistant" ? m.voice_overlay_error_action_resubmit() : m.voice_overlay_error_action_retry_send()) : m.voice_overlay_error_action_redo());
  /** Esc 出口：带文本时按下等于丢弃保留的文本（放弃），否则只是收起提示。 */
  const errorEscAction = $derived($appState.errorText ? m.voice_overlay_esc_discard() : m.voice_overlay_esc_close());
  const processingLabel = $derived(
    $appState.purpose === "assistant" ? m.voice_overlay_processing_assistant() : $appState.processingStep
      ? `${$appState.processingStep.displayName} · ${$appState.processingStep.index}/${$appState.processingStep.total}`
      : m.voice_overlay_processing(),
  );
</script>

<!--
  两种样式（overlay.style）：
  - card：480×120 窗口内铺满一条圆角悬浮条；非按钮区 pointerdown 由后端 drag_overlay 跟着光标整窗拖动。
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
    class:render-paused={!renderActive}
    onpointerenter={onPointerEnter}
    onpointerdown={onDragPointerDown}
    onpointerup={onPointerUp}
  >
    <div
      class="flex max-w-full items-center gap-2.5 rounded-full bg-kotone-deep/96 py-2 pr-4 pl-3.5 ring-1 ring-kotone-cyan/35"
      style:width="fit-content"
    >
      {#if $appState.channel}
        <!-- 当前发送频道徽标（仅非默认频道显示）；id 变化时重放弹跳动画提示「已切换」 -->
        {#key $appState.channel.id}
          <span
            class="channel-badge shrink-0 rounded-full bg-kotone-violet/30 px-2 py-0.5 text-[10px] leading-tight font-semibold whitespace-nowrap text-kotone-violet ring-1 ring-kotone-violet/50"
            title={m.voice_overlay_channel_title({ name: $appState.channel.displayName })}
          >
            {$appState.channel.displayName}
          </span>
        {/key}
      {/if}
      {#if $appState.ppNotice && $appState.state !== "idle"}
        {#key $appState.ppNotice.id}
          <span
            class="pp-notice shrink-0 rounded-full bg-kotone-cyan/15 px-2.5 py-0.5 text-[10px] leading-tight font-semibold whitespace-nowrap text-kotone-cyan ring-1 ring-kotone-cyan/40"
            in:fade={{ duration: 120 }}
          >
            {$appState.ppNotice.text}
          </span>
        {/key}
      {/if}
      {#if $appState.state === "listening"}
        <!-- 收到真实 partial 后立即上屏；尚无 partial（含非流式引擎）时显示声波。 -->
        <span class="mic-breath h-2.5 w-2.5 shrink-0 rounded-full bg-kotone-cyan"></span>
        {#if $appState.warningMessage}
          <p class="max-w-[360px] truncate text-xs text-amber-300" in:fade={{ duration: 150 }}>
            {$appState.warningMessage}
          </p>
        {:else if $appState.partialText}
          <p
            class="ellipsis-head max-w-[340px] truncate text-sm text-white"
            in:fade={{ duration: 150 }}
          >
            {$appState.partialText}&#x200E;
          </p>
        {:else}
          <Waveform level={$appState.level} bars={12} active={renderActive} />
          {#if $appState.purpose === "assistant"}
            <span class="truncate text-xs text-kotone-cyan">{assistantListeningHint}</span>
          {/if}
        {/if}
      {:else if $appState.state === "transcribing"}
        <span class="spinner inline-block h-3.5 w-3.5 shrink-0 rounded-full"></span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-kotone-violet"
          in:fade={{ duration: 150 }}
        >
          {$appState.partialText || m.voice_overlay_transcribing()}&#x200E;
        </p>
      {:else if $appState.state === "processing"}
        <span class="spinner inline-block h-3.5 w-3.5 shrink-0 rounded-full"></span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-kotone-cyan"
          in:fade={{ duration: 150 }}
        >
          {processingLabel}&#x200E;
        </p>
      {:else if $appState.state === "preview"}
        <p
          class="ellipsis-head max-w-[260px] truncate text-sm text-white"
          in:fade={{ duration: 150 }}
        >
          {$appState.finalText}&#x200E;
        </p>
        <span class="shrink-0 text-[10px] whitespace-nowrap text-white/40">
          {previewHint}
        </span>
        {#if mouseInteractive}
          <button
            class="shrink-0 rounded-full bg-kotone-pink px-2.5 py-0.5 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95"
            onclick={() => void onConfirm()}
            disabled={sendPending}
          >
            {confirmLabel}
          </button>
        {/if}
      {:else if $appState.state === "sending"}
        <span class="send-pulse inline-block h-2.5 w-2.5 shrink-0 rounded-full bg-kotone-pink"></span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-white/70"
          in:fade={{ duration: 150 }}
        >
          {$appState.finalText || m.voice_overlay_state_sending()}&#x200E;
        </p>
      {:else if $appState.state === "success"}
        <!-- 发送后短暂显示结果（随后按 overlay.visibility 规则隐藏/驻留） -->
        <span class="shrink-0 text-sm font-bold text-kotone-cyan">✓</span>
        <p
          class="ellipsis-head max-w-[380px] truncate text-sm text-white"
          in:fade={{ duration: 150 }}
        >
          {$appState.purpose === "assistant" ? m.voice_overlay_success_assistant() : m.voice_overlay_state_success()} · {$appState.finalText}&#x200E;
        </p>
      {:else if $appState.state === "error"}
        <span class="shrink-0 text-sm font-bold text-kotone-pink">✗</span>
        <p class="max-w-[220px] truncate text-sm text-kotone-pink" title={displayError}>
          {displayError}
        </p>
        {#if mouseInteractive && $appState.errorText}
          <button
            class="shrink-0 rounded-full bg-kotone-pink/80 px-2.5 py-0.5 text-[11px] font-semibold text-white transition hover:brightness-110 active:scale-95"
            onclick={() => void onRetry()}
            disabled={sendPending}
          >
            {m.voice_action_retry()}
          </button>
        {/if}
        <!-- Error 的退出一律走键盘：无文本时没有任何鼠标可做的事，不给按钮 -->
        <span class="shrink-0 text-[10px] whitespace-nowrap text-white/40">
          {$appState.purpose === "assistant" ? assistantHotkeyLabel : hotkeyLabel} {errorAction} · Esc {errorEscAction}
        </span>
      {:else}
        <!-- idle：热键切换文字处理时短暂显示提示（托盘手动唤起时显示待机文案） -->
        {#if notReadyHint}
          <span class="h-2 w-2 shrink-0 rounded-full bg-amber-300"></span>
          <p class="shrink-0 text-sm font-semibold whitespace-nowrap text-amber-300" data-testid="overlay-not-ready-title">{notReadyHint.title}</p>
          <p class="max-w-[300px] truncate text-xs text-white/65" title={notReadyHint.detail}>{notReadyHint.detail}</p>
        {:else if $appState.ppNotice}
          <p class="shrink-0 text-sm whitespace-nowrap text-kotone-cyan">
            {$appState.ppNotice.text}
          </p>
        {:else}
          <span class="h-2 w-2 shrink-0 rounded-full bg-kotone-cyan/60"></span>
          <p class="shrink-0 text-sm whitespace-nowrap text-white/70">{m.voice_overlay_idle_capsule()}</p>
        {/if}
      {/if}
    </div>
  </div>
{:else}
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="flex h-full items-stretch select-none {draggable && !clickThrough ? 'cursor-move' : ''}"
  class:render-paused={!renderActive}
  onpointerenter={onPointerEnter}
  onpointerdown={onDragPointerDown}
  onpointerup={onPointerUp}
>
  <div
    class="flex w-full items-center gap-3 rounded-2xl bg-kotone-deep/96 px-4 py-2 ring-1 ring-inset ring-kotone-cyan/40"
  >
    {#if $appState.channel}
      {#key $appState.channel.id}
        <span
          class="channel-badge shrink-0 rounded-full bg-kotone-violet/30 px-2 py-1 text-[10px] leading-tight font-semibold whitespace-nowrap text-kotone-violet ring-1 ring-kotone-violet/50"
          title={m.voice_overlay_channel_title({ name: $appState.channel.displayName })}
        >
          {$appState.channel.displayName}
        </span>
      {/key}
    {/if}
    {#if $appState.ppNotice && $appState.state !== "idle"}
      {#key $appState.ppNotice.id}
        <span
          class="pp-notice shrink-0 rounded-full bg-kotone-cyan/15 px-2.5 py-1 text-[10px] leading-tight font-semibold whitespace-nowrap text-kotone-cyan ring-1 ring-kotone-cyan/40"
          in:fade={{ duration: 120 }}
        >
          {$appState.ppNotice.text}
        </span>
      {/key}
    {/if}
    {#if $appState.state === "listening"}
      <!-- 收到真实 partial 后立即上屏；尚无 partial（含非流式引擎）时显示声波。 -->
      <span class="mic-breath flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-kotone-cyan/12" in:fade={{ duration: 150 }}>
        <svg viewBox="0 0 24 24" fill="none" stroke="#00e5ff" stroke-width="2" class="h-4.5 w-4.5">
          <rect x="9" y="2" width="6" height="12" rx="3" />
          <path d="M5 10a7 7 0 0 0 14 0M12 19v3" stroke-linecap="round" />
        </svg>
      </span>
      {#if $appState.warningMessage}
        <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
          <p class="text-[11px] leading-tight font-semibold text-amber-300">{m.voice_overlay_manual_end()}</p>
          <p class="mt-0.5 truncate text-xs text-white/65">{$appState.warningMessage}</p>
        </div>
      {:else if $appState.partialText}
        <div class="min-w-0 flex-1">
          <p class="text-[11px] leading-tight text-kotone-cyan/80">{$appState.purpose === "assistant" ? assistantListeningHint : stateLabel.listening}</p>
          <div bind:this={textScrollEl} class="kotone-scroll mt-0.5 max-h-14 overflow-y-auto pr-1">
            <p class="text-sm leading-snug break-all text-white">
              {$appState.partialText}
            </p>
          </div>
        </div>
      {:else}
        <!-- 第一条 partial 到达前保持声波；非流式引擎会始终停留在这里。 -->
        <div class="flex min-w-0 flex-1 items-center justify-center {$appState.purpose === 'assistant' ? 'flex-col gap-1' : ''}" in:fade={{ duration: 150 }}>
          <Waveform level={$appState.level} active={renderActive} />
          {#if $appState.purpose === "assistant"}
            <p class="text-[11px] leading-tight text-kotone-cyan/80">{assistantListeningHint}</p>
          {/if}
        </div>
      {/if}
    {:else if $appState.state === "transcribing"}
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="flex items-center gap-2 text-sm text-kotone-violet">
          <span class="spinner inline-block h-3.5 w-3.5 rounded-full"></span>
          {m.voice_overlay_transcribing()}
        </p>
        {#if $appState.partialText}
          <p class="mt-0.5 truncate text-xs text-white/50">{$appState.partialText}</p>
        {/if}
      </div>
    {:else if $appState.state === "processing"}
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="flex items-center gap-2 text-sm text-kotone-cyan">
          <span class="spinner inline-block h-3.5 w-3.5 rounded-full"></span>
          {processingLabel}
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
          {m.voice_overlay_press_again({ hint: previewHint })}
        </p>
        {#if actionError}
          <p class="truncate text-[11px] text-kotone-pink">{actionError}</p>
        {/if}
      </div>
      {#if mouseInteractive}
        <div class="flex shrink-0 flex-col gap-1.5">
          <button
            class="rounded-lg bg-kotone-pink px-3 py-1 text-xs font-semibold text-white shadow-glow-pink transition hover:brightness-110 active:scale-95"
            onclick={() => void onConfirm()}
            disabled={sendPending}
          >
            {confirmLabel} ⏎
          </button>
          <button
            class="rounded-lg bg-white/10 px-3 py-1 text-xs text-white/75 transition hover:bg-white/20 active:scale-95"
            onclick={() => void onCancel()}
          >
            {m.voice_overlay_cancel_esc()}
          </button>
        </div>
      {/if}
    {:else if $appState.state === "sending"}
      <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
        <p class="flex items-center gap-2 text-sm font-semibold text-kotone-pink">
          <span class="send-pulse inline-block h-3 w-3 rounded-full bg-kotone-pink"></span>
          {m.voice_overlay_state_sending()}
        </p>
        <p class="mt-0.5 truncate text-xs text-white/50">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "success"}
      <!-- 已发送：Kotone 贴纸弹出 + 气泡 -->
      <img
        src={stickerProud}
        alt={m.voice_overlay_sticker_proud_alt()}
        class="h-14 w-14 shrink-0 object-contain"
        in:scale={{ duration: 220, start: 0.5 }}
      />
      <div class="min-w-0 flex-1" in:fly={{ y: 6, duration: 200 }}>
        <div class="bubble inline-block max-w-full px-3 py-1.5">
          <p class="text-sm font-semibold text-white">{$appState.purpose === "assistant" ? m.voice_overlay_success_assistant() : m.voice_overlay_success_sent()}</p>
        </div>
        <p class="mt-1 truncate text-xs text-white/55">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "error"}
      <!-- 出错：amazed 贴纸 + 错误摘要 -->
      <img
        src={stickerAmazed}
        alt={m.voice_overlay_sticker_amazed_alt()}
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
          <p class="mt-0.5 truncate text-[11px] text-white/50">{m.voice_overlay_kept_text({ text: $appState.errorText })}</p>
        {/if}
        {#if $appState.needsElevation}
          <p class="mt-0.5 text-[11px] text-kotone-cyan/85">
            {m.voice_overlay_needs_elevation()}
          </p>
        {/if}
      </div>
      <div class="flex shrink-0 flex-col items-end gap-1">
        {#if mouseInteractive && $appState.errorText}
          <button
            class="rounded-lg bg-kotone-pink/80 px-2.5 py-1 text-xs font-semibold text-white transition hover:brightness-110 active:scale-95"
            onclick={() => void onRetry()}
            disabled={sendPending}
          >
            {m.voice_action_retry()}
          </button>
        {/if}
        <!-- Error 的退出一律走键盘：无文本时没有任何鼠标可做的事，不给按钮 -->
        <span class="text-[11px] whitespace-nowrap text-white/45">
          {m.voice_overlay_error_hint({ hotkey: $appState.purpose === "assistant" ? assistantHotkeyLabel : hotkeyLabel, action: errorAction, escAction: errorEscAction })}
        </span>
      </div>
    {:else}
      <!-- idle（托盘手动唤起悬浮条时可见；正常流程会自动隐藏窗口） -->
      {#if notReadyHint}
        <div class="min-w-0 flex-1" in:fade={{ duration: 150 }} role="status">
          <p class="text-sm font-semibold text-amber-300" data-testid="overlay-not-ready-title">{notReadyHint.title}</p>
          <p class="truncate text-[11px] text-white/60" title={notReadyHint.detail}>{notReadyHint.detail}</p>
        </div>
      {:else if $appState.ppNotice}
        <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
          <p class="text-sm font-semibold text-kotone-cyan">{$appState.ppNotice.text}</p>
          <p class="text-[11px] text-white/40">{m.voice_overlay_pp_updated()}</p>
        </div>
      {:else}
        <div class="min-w-0 flex-1" in:fade={{ duration: 150 }}>
          <p class="text-sm font-medium text-white/80">{m.voice_overlay_idle_title()}</p>
          <p class="text-[11px] text-white/40">{m.voice_overlay_idle_hint()}</p>
        </div>
      {/if}
    {/if}
  </div>
</div>
{/if}

<style>
  /* 窗口隐藏后仍会收到状态/电平事件，显式停掉整个子树的动画与过渡。 */
  .render-paused :global(*) {
    animation-play-state: paused !important;
    transition: none !important;
  }

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

  /* 频道徽标：切换瞬间弹跳提示 */
  .channel-badge {
    animation: kotone-badge-pop 0.4s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  @keyframes kotone-badge-pop {
    0% {
      transform: scale(0.5);
      opacity: 0;
    }
    100% {
      transform: scale(1);
      opacity: 1;
    }
  }

  /* 保持小光晕静态，只改变透明度，避免每帧重绘阴影。 */
  .mic-breath {
    box-shadow: 0 0 6px rgba(0, 229, 255, 0.35);
    animation: kotone-mic-breath 1.8s ease-in-out infinite;
  }
  @keyframes kotone-mic-breath {
    0%,
    100% {
      opacity: 0.65;
    }
    50% {
      opacity: 1;
    }
  }

  /* 发送中：静态品红光晕 + 透明度脉冲 */
  .send-pulse {
    box-shadow: 0 0 4px rgba(255, 45, 120, 0.5);
    animation: kotone-send-pulse 0.9s ease-in-out infinite;
  }
  @keyframes kotone-send-pulse {
    0%,
    100% {
      opacity: 0.6;
    }
    50% {
      opacity: 1;
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
