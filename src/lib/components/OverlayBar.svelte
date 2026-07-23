<script lang="ts">
  /*
   * 悬浮录音条（docs/development.md §3.4、§5.2 OverlayBar）。
   * 状态驱动 UI：orchestrator 是唯一状态所有者，这里只渲染 appState。
   *
   *   listening    波形 + 流式 partial 实时上屏（无 partial 时「聆听中…」）
   *   transcribing 「转写中…」
   *   preview      可编辑文本框 + 确认 / 取消
   *   sending      发送动画
   *   success      「收到，已发送！✨」toast
   *   error        错误消息 + 重试 / 关闭
   */
  import { fade, fly } from "svelte/transition";
  import { appState } from "../stores/state";
  import { confirmSend, cancelSession, simulateSend } from "../ipc";
  import Waveform from "./Waveform.svelte";

  /** 各状态指示点颜色 */
  const dotColor: Record<string, string> = {
    idle: "bg-white/40",
    listening: "bg-kotone-cyan",
    transcribing: "bg-kotone-violet",
    preview: "bg-kotone-cyan",
    sending: "bg-kotone-violet",
    success: "bg-kotone-pink",
    error: "bg-kotone-pink",
  };

  const stateLabel: Record<string, string> = {
    idle: "Kotone 待机",
    listening: "聆听中",
    transcribing: "转写中",
    preview: "预览确认",
    sending: "发送中",
    success: "已发送",
    error: "出错了",
  };

  /** preview 态的可编辑文本（进入 preview 时同步 finalText） */
  let editText = $state("");
  /** 操作中的瞬时错误提示（如 confirm 调用失败） */
  let actionError = $state("");
  /** 重试后的本地提示 */
  let actionHint = $state("");

  let textScrollEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    if ($appState.state === "preview") {
      editText = $appState.finalText;
      actionError = "";
      actionHint = "";
    }
  });

  // partial 更新时滚动到底部（流式上屏）
  $effect(() => {
    void $appState.partialText;
    if (textScrollEl) textScrollEl.scrollTop = textScrollEl.scrollHeight;
  });

  async function onConfirm() {
    actionError = "";
    try {
      await confirmSend(editText.trim() ? editText : undefined);
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
  480×120 窗口内铺满一条圆角悬浮条；data-tauri-drag-region 使空白处可拖动窗口
-->
<div
  class="flex h-full items-stretch p-2 select-none"
  data-tauri-drag-region
>
  <div
    class="flex w-full items-center gap-3 rounded-2xl bg-kotone-deep/90 px-4 py-2 shadow-[0_0_24px_rgba(0,229,255,0.15)] ring-1 ring-kotone-cyan/40"
    data-tauri-drag-region
  >
    <!-- 状态指示点 -->
    <span
      class="inline-block h-2.5 w-2.5 shrink-0 rounded-full transition-colors duration-300 {dotColor[
        $appState.state
      ]}"
      class:animate-pulse={$appState.state === "listening" || $appState.state === "sending"}
    ></span>

    {#if $appState.state === "listening"}
      <!-- 聆听：波形 + 流式 partial 上屏 -->
      <div class="shrink-0" in:fade={{ duration: 150 }}>
        <Waveform level={$appState.level} />
      </div>
      <div class="min-w-0 flex-1">
        <p class="text-[11px] leading-tight text-kotone-cyan/80">{stateLabel.listening}</p>
        <div
          bind:this={textScrollEl}
          class="kotone-scroll mt-0.5 max-h-14 overflow-y-auto pr-1"
        >
          {#if $appState.partialText}
            {#key $appState.partialText}
              <p
                class="text-sm leading-snug break-all text-white"
                in:fade={{ duration: 180 }}
              >
                {$appState.partialText}
              </p>
            {/key}
          {:else}
            <p class="text-sm text-white/45">聆听中…</p>
          {/if}
        </div>
      </div>
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
      <!-- 预览：可编辑 + 确认/取消 -->
      <div class="flex min-w-0 flex-1 flex-col gap-1.5" in:fade={{ duration: 150 }}>
        <textarea
          bind:value={editText}
          rows="2"
          spellcheck="false"
          class="kotone-scroll w-full resize-none rounded-lg bg-white/8 px-2.5 py-1.5 text-sm leading-snug text-white ring-1 ring-kotone-cyan/30 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/70"
          placeholder="确认识别文本，可直接编辑…"
          onkeydown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void onConfirm();
            } else if (e.key === "Escape") {
              void onCancel();
            }
          }}
        ></textarea>
        {#if actionError}
          <p class="truncate text-[11px] text-kotone-pink">{actionError}</p>
        {/if}
      </div>
      <div class="flex shrink-0 flex-col gap-1.5">
        <button
          class="rounded-lg bg-kotone-cyan px-3 py-1 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95"
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
        <p class="flex items-center gap-1.5 text-sm text-kotone-violet">
          发送中
          <span class="send-dots inline-flex gap-0.5">
            <i></i><i></i><i></i>
          </span>
        </p>
        <p class="mt-0.5 truncate text-xs text-white/50">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "success"}
      <div class="min-w-0 flex-1" in:fly={{ y: 6, duration: 200 }}>
        <p
          class="bg-gradient-to-r from-kotone-pink to-kotone-cyan bg-clip-text text-sm font-bold text-transparent"
        >
          收到，已发送！✨
        </p>
        <p class="mt-0.5 truncate text-xs text-white/55">{$appState.finalText}</p>
      </div>
    {:else if $appState.state === "error"}
      <div class="min-w-0 flex-1" in:fly={{ y: 6, duration: 200 }}>
        <p class="line-clamp-2 text-[13px] leading-snug break-all font-medium text-kotone-pink" title={displayError}>{displayError}</p>
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

<style>
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

  /* 发送中的三点跳动 */
  .send-dots i {
    width: 4px;
    height: 4px;
    border-radius: 9999px;
    background: #7b2fff;
    animation: kotone-hop 1s ease-in-out infinite;
  }
  .send-dots i:nth-child(2) {
    animation-delay: 0.15s;
  }
  .send-dots i:nth-child(3) {
    animation-delay: 0.3s;
  }
  @keyframes kotone-hop {
    0%,
    100% {
      transform: translateY(0);
      opacity: 0.4;
    }
    50% {
      transform: translateY(-3px);
      opacity: 1;
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
