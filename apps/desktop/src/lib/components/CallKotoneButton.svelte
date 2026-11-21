<script lang="ts">
  // 聊天页的呼叫按钮（ADR 028）：和呼叫键一样，按一次就在右下角气泡里听你说，说完自动交给她；
  // 她在听时再按一次立刻提交。用本地识别且还没启动时，点一下会先启动本地识别，准备好就开始听。
  import { companionName } from "$lib/stores/companion";
  import { Mic } from "@lucide/svelte";
  import { callKotone } from "../assistant";
  import { hotkeyLabel } from "../hotkeyCombo";
  import { companionBindings, isTauri, startRuntime } from "../ipc";
  import { appState } from "../stores/state";
  import { asrStore } from "../stores/runtime";
  import { errText, settingsStore, toast } from "../stores/ui";
  import { wakePhrases } from "../wake";

  let { size = "sm", ready = true, testid = "call-kotone", onSetCallKey }: {
    /** icon：输入框里发送键旁边的图标（悬停看快捷键）；sm：小按钮；lg：聊天页中间的大按钮。 */
    size?: "icon" | "sm" | "lg";
    /** AI 连接选好了没有。 */
    ready?: boolean;
    testid?: string;
    /** 没有呼叫键时，说明里给一个“设一个呼叫键 →”。 */
    onSetCallKey?: () => void;
  } = $props();

  const name = $derived($companionName);
  const hotkey = $derived(companionBindings($settingsStore?.assistant)?.hotkey.trim() ?? "");
  const wake = $derived($settingsStore?.assistant.wake);
  const wakeWord = $derived(wake?.enabled ? (wakePhrases.find((p) => p.value === wake.phrase)?.label ?? wake.phrase) : "");
  const listening = $derived($appState.purpose === "assistant" && !$appState.legacyReminder && $appState.state === "listening");
  const asr = $derived($asrStore);
  const preparing = $derived(asr === "local-starting" || asr === "online-preparing");
  const blocked = $derived(!isTauri ? "仅限桌面版" : !ready ? "先配好 AI 连接" : "");
  let calling = $state(false);
  const label = $derived(listening ? `${name}在听…` : calling && preparing ? "正在准备…" : `呼叫${name}`);
  const hint = $derived(blocked
    || (listening ? "说完了？再按一次直接交给她"
      : asr === "local-stopped" || asr === "local-error" ? "点一下会先启动本地识别，准备好就开始听"
      : hotkey && wakeWord ? `按 ${hotkeyLabel(hotkey)} 或者说“${wakeWord}”`
      : hotkey ? `或在游戏里按 ${hotkeyLabel(hotkey)}`
      : wakeWord ? `或者说“${wakeWord}”`
      : ""));

  /** 图标按钮悬停时的说明：呼叫琴音（快捷键 ~）……（rc2 玩家反馈：输入框里只留一个图标，悬停看快捷键）。 */
  const iconTitle = $derived(blocked ? `呼叫${name}：${blocked}`
    : listening ? `${name}在听…说完了？再按一次直接交给她`
    : calling && preparing ? "正在准备…"
    : [hotkey ? `呼叫${name}（快捷键 ${hotkeyLabel(hotkey)}）` : `呼叫${name}（还没设快捷键：设置 → 怎么叫她）`,
       wakeWord ? `也可以说“${wakeWord}”` : "",
       asr === "local-stopped" || asr === "local-error" ? "点一下会先启动本地识别" : ""].filter(Boolean).join(" · "));

  async function call() {
    if (blocked || calling) return;
    calling = true;
    try {
      // 语音识别没准备好：先启动（本地识别）或等它准备好（在线识别），再开始听。
      if (asr !== "local-running" && asr !== "online-ready") await startRuntime();
      await callKotone();
    } catch (error) {
      toast(false, `呼叫失败：${errText(error)}`);
    } finally {
      calling = false;
    }
  }
</script>

{#if size === "lg"}
  <div class="flex flex-col items-center gap-2" data-testid={`${testid}-block`}>
    <button type="button" data-testid={testid} disabled={!!blocked || calling} aria-pressed={listening}
      class="flex h-14 items-center gap-2.5 rounded-full px-7 text-base font-semibold shadow-sm transition
        disabled:cursor-not-allowed disabled:opacity-50
        {listening ? 'bg-brand-pink text-white' : 'bg-primary text-primary-foreground hover:brightness-110'}"
      onclick={() => void call()}>
      <Mic class="size-5 {listening || calling ? 'animate-pulse' : ''}" />
      {label}
    </button>
    <p class="m-0 text-xs text-muted-foreground" data-testid={`${testid}-hint`}>
      {hint}
      {#if !blocked && !listening && !hotkey && onSetCallKey}
        {hint ? " · " : ""}<button type="button" class="text-primary hover:underline" data-testid={`${testid}-set-key`} onclick={onSetCallKey}>设一个呼叫键 →</button>
      {/if}
    </p>
  </div>
{:else if size === "icon"}
  <button type="button" data-testid={testid} disabled={!!blocked || calling} aria-pressed={listening}
    title={iconTitle} aria-label={iconTitle}
    class="flex size-8 shrink-0 items-center justify-center rounded-full transition
      disabled:cursor-not-allowed disabled:opacity-50
      {listening ? 'bg-brand-pink/15 text-brand-pink' : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
    onclick={() => void call()}>
    <Mic class="size-4 {listening || calling ? 'animate-pulse' : ''}" />
    <span class="sr-only">{listening ? "在听…" : `呼叫${name}`}</span>
  </button>
{:else}
  <button type="button" data-testid={testid} disabled={!!blocked || calling} aria-pressed={listening}
    title={hint}
    class="flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-medium transition
      disabled:cursor-not-allowed disabled:opacity-50
      {listening ? 'bg-brand-pink/15 text-brand-pink' : 'bg-muted/70 text-foreground hover:bg-muted'}"
    onclick={() => void call()}>
    <Mic class="size-3.5 {listening || calling ? 'animate-pulse' : ''}" />
    {listening ? "在听…" : calling && preparing ? "准备中…" : `呼叫${name}`}
    {#if hotkey && !blocked && !listening}<span class="text-[10px] text-muted-foreground">{hotkeyLabel(hotkey)}</span>{/if}
  </button>
{/if}
