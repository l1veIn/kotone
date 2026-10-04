<script lang="ts">
  import { onMount } from "svelte";
  import { isTauri } from "../../../../lib/ipc";
  import { createAssistantVoiceControl } from "../../../../lib/assistantVoice";
  import { findHotkeyConflict, keyboardEventCombo } from "../../../../lib/hotkeyCombo";
  import { appState } from "../../../../lib/stores/state";
  import { runtimeStore } from "../../../../lib/stores/runtime";
  import { errText, settingsStore } from "../../../../lib/stores/ui";

  let { ready, active, submitting, onOpenSettings, onBusyChange }: {
    ready: boolean; active: boolean; submitting: boolean;
    onOpenSettings: () => void; onBusyChange: (busy: boolean) => void;
  } = $props();
  let error = $state("");
  const voice = createAssistantVoiceControl((failure) => { error = errText(failure); });
  const configuredMode = $derived($settingsStore?.assistant.interactionMode ?? "push-to-talk");
  const mode = $derived($voice.phase === "idle" ? configuredMode : $voice.mode);
  const hotkey = $derived($settingsStore?.assistant.hotkey.trim() ?? "");
  const inputBusy = $derived(!["idle", "success", "error"].includes($appState.state));
  const own = $derived($voice.phase !== "idle");
  const blocked = $derived(!isTauri ? "语音输入仅限桌面版"
    : $runtimeStore?.phase !== "running" ? "请先启动 Kotone"
    : !ready ? "请先选择 AI 连接"
    : submitting ? "正在创建提醒"
    : inputBusy && !own ? ($appState.purpose === "assistant" ? "助手语音尚未结束" : "请先结束当前输入") : "");
  const label = $derived($voice.phase === "starting" ? "开启麦克风…"
    : $voice.phase === "cancelling" ? "取消中…"
    : $voice.phase === "working" ? "处理中…"
    : $voice.phase === "preview" ? "确认创建"
    : $voice.phase === "listening" ? (mode === "push-to-talk" ? "松开提交" : "结束录音")
    : mode === "push-to-talk" ? "按住说话" : "语音输入");
  const disabled = $derived(!!blocked && !own || ["working", "cancelling"].includes($voice.phase));
  const hideHint = $derived((!ready && isTauri) || (!blocked && mode === "push-to-talk"));
  let pointer: number | null = null;
  let heldKey: string | null = null;
  let globalKey: string | null = null;

  $effect(() => { voice.observe($appState); });
  $effect(() => { onBusyChange(own); });
  $effect(() => { if (!active) cancel(); });

  function start(held: boolean) {
    if (blocked || own || !active) return;
    error = "";
    voice.begin(configuredMode, held);
  }
  function cancel() {
    pointer = null;
    heldKey = null;
    globalKey = null;
    voice.cancel();
  }
  function down(event: PointerEvent) {
    if (event.button !== 0 || disabled || mode !== "push-to-talk" || own) return;
    event.preventDefault();
    const button = event.currentTarget as HTMLButtonElement;
    button.focus();
    pointer = event.pointerId;
    button.setPointerCapture(event.pointerId);
    start(true);
  }
  function up(event: PointerEvent) {
    if (pointer !== event.pointerId) return;
    pointer = null;
    voice.release();
    const button = event.currentTarget as HTMLButtonElement;
    if (button.hasPointerCapture(event.pointerId)) button.releasePointerCapture(event.pointerId);
  }
  function click(event: MouseEvent) {
    if (disabled) return;
    if ($voice.phase === "preview") { voice.confirm(); return; }
    if (mode === "push-to-talk") return; // Pointer/key down and up own hold semantics.
    if ($voice.phase === "listening") voice.finish();
    else if ($voice.phase === "idle") start(false);
  }
  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") { event.preventDefault(); cancel(); return; }
    if ([" ", "Enter"].includes(event.key)) {
      const combo = keyboardEventCombo(event);
      // The native hook has already seen this key. Do not also activate the DOM button.
      if (combo && findHotkeyConflict($settingsStore, "assistant-voice-button", combo)) {
        event.preventDefault();
        globalKey = event.key;
        return;
      }
      if (event.repeat) { event.preventDefault(); return; }
    }
    if (mode !== "push-to-talk" || $voice.phase === "preview" || ![" ", "Enter"].includes(event.key)) return;
    event.preventDefault();
    if (event.repeat || heldKey || disabled || own) return;
    heldKey = event.key;
    start(true);
  }
  function keyup(event: KeyboardEvent) {
    if (event.key === globalKey) { event.preventDefault(); globalKey = null; return; }
    if (event.key !== heldKey) return;
    event.preventDefault();
    heldKey = null;
    voice.release();
  }
  onMount(() => {
    const hidden = () => { if (document.hidden) cancel(); };
    window.addEventListener("blur", cancel);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("blur", cancel);
      document.removeEventListener("visibilitychange", hidden);
      voice.dispose();
      onBusyChange(false);
    };
  });
</script>

<div class="min-w-0 text-right" data-testid="assistant-voice-input">
  <div class="flex flex-wrap items-center justify-end gap-2">
    {#if own}<button type="button" class="secondary" disabled={$voice.phase === "cancelling"} onclick={cancel} aria-label="取消语音输入">取消</button>{/if}
    <button type="button" data-testid="assistant-voice-button" aria-label={label}
      aria-describedby="assistant-voice-hint" aria-pressed={$voice.phase === "listening"} {disabled}
      class="inline-flex touch-none select-none items-center gap-2 rounded-lg px-3 py-2 text-xs font-semibold ring-1 transition disabled:cursor-not-allowed disabled:opacity-40
        {own ? 'bg-kotone-pink/15 text-kotone-pink ring-kotone-pink/45' : 'bg-kotone-cyan/10 text-kotone-cyan ring-kotone-cyan/35 hover:bg-kotone-cyan/20'}"
      onpointerdown={down} onpointerup={up} onpointercancel={cancel}
      onlostpointercapture={() => { if (pointer !== null) cancel(); }}
      onkeydown={keydown} onkeyup={keyup} onclick={click}
      onblur={() => { if (pointer !== null || heldKey !== null) cancel(); }}>
      <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" class="h-4 w-4"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/></svg>
      {label}
    </button>
  </div>
  <div class="mt-1.5 flex items-center justify-end gap-1 whitespace-nowrap text-[11px] leading-relaxed text-white/45">
    <button type="button" class="min-w-0 truncate hover:text-kotone-cyan" onclick={onOpenSettings} data-testid="assistant-voice-hotkey"
      title={hotkey ? `快捷键：${hotkey}` : "设置快捷键"}>
      {hotkey ? `快捷键：${hotkey}` : "设置快捷键"}
    </button>
    {#if !hideHint}<span aria-hidden="true" class="shrink-0">·</span>{/if}
    <p id="assistant-voice-hint" class="shrink-0" class:sr-only={hideHint}>
      {blocked || (mode === "push-to-talk" ? "按住按钮说话，松开提交；聚焦后也可用空格或 Enter" : mode === "dictation" ? "结束录音后确认" : "说完自动提交")}
    </p>
  </div>
  {#if own && $appState.purpose === "assistant" && ($appState.finalText || $appState.partialText)}
    <p class="mt-2 max-w-80 break-words text-xs text-kotone-cyan" data-testid="assistant-voice-transcript">{$appState.finalText || $appState.partialText}</p>
  {/if}
  {#if error}<p role="alert" class="mt-2 max-w-80 text-xs text-kotone-pink">{error}</p>{/if}
</div>
