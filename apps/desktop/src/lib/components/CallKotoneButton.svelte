<script lang="ts">
  // 呼叫琴音（首页、聊天窗口）：和热键一样，按一次就在右下角气泡里听你说，说完自动交给她；
  // 她在听时再按一次立刻提交。相当于按键版的语音唤醒，没有“按住说话”。
  import { companionName } from "$lib/stores/companion";
  import { Mic } from "@lucide/svelte";
  import { callKotone } from "../assistant";
  import { hotkeyLabel } from "../hotkeyCombo";
  import { isTauri } from "../ipc";
  import { appState } from "../stores/state";
  import { runtimeStore } from "../stores/runtime";
  import { errText, settingsStore, toast } from "../stores/ui";

  let { size = "sm", ready = true, testid = "call-kotone" }: {
    /** sm：聊天输入框里的小按钮；lg：首页的大按钮。 */
    size?: "sm" | "lg";
    /** AI 连接选好了没有。 */
    ready?: boolean;
    testid?: string;
  } = $props();

  const name = $derived($companionName);
  const hotkey = $derived($settingsStore?.assistant.hotkey.trim() ?? "");
  const listening = $derived($appState.purpose === "assistant" && $appState.state === "listening");
  const blocked = $derived(!isTauri ? "仅限桌面版"
    : $runtimeStore?.phase !== "running" ? "先点右上角「启动」"
    : !ready ? "先选好 AI 连接" : "");
  const hint = $derived(blocked || (listening ? "说完了？再按一次直接交给她"
    : hotkey ? `或在游戏里按 ${hotkeyLabel(hotkey)}` : "也可以在「设置」里给它配一个热键"));
  let calling = $state(false);

  async function call() {
    if (blocked || calling) return;
    calling = true;
    try { await callKotone(); }
    catch (error) { toast(false, `呼叫失败：${errText(error)}`); }
    finally { calling = false; }
  }
</script>

{#if size === "lg"}
  <div class="flex flex-col items-center gap-2" data-testid={`${testid}-block`}>
    <button type="button" data-testid={testid} disabled={!!blocked || calling} aria-pressed={listening}
      class="flex h-14 items-center gap-2.5 rounded-full px-7 text-base font-semibold shadow-sm transition
        disabled:cursor-not-allowed disabled:opacity-50
        {listening ? 'bg-brand-pink text-white' : 'bg-primary text-primary-foreground hover:brightness-110'}"
      onclick={() => void call()}>
      <Mic class="size-5 {listening ? 'animate-pulse' : ''}" />
      {listening ? `${name}在听…` : `呼叫${name}`}
    </button>
    <p class="m-0 text-xs text-muted-foreground" data-testid={`${testid}-hint`}>{hint}</p>
  </div>
{:else}
  <button type="button" data-testid={testid} disabled={!!blocked || calling} aria-pressed={listening}
    title={hint}
    class="flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-medium transition
      disabled:cursor-not-allowed disabled:opacity-50
      {listening ? 'bg-brand-pink/15 text-brand-pink' : 'bg-muted/70 text-foreground hover:bg-muted'}"
    onclick={() => void call()}>
    <Mic class="size-3.5 {listening ? 'animate-pulse' : ''}" />
    {listening ? "在听…" : `呼叫${name}`}
    {#if hotkey && !blocked && !listening}<span class="text-[10px] text-muted-foreground">{hotkeyLabel(hotkey)}</span>{/if}
  </button>
{/if}
