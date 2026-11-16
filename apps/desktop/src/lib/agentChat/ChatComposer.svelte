<script lang="ts">
  // 输入框：Enter 发送、Shift+Enter 换行、输入法组字时不发送；回答中显示“停止”，仍可发送（插话）。
  // 外形参照 DeepSeek Harness 的输入卡（InputBar，MIT）：大圆角卡片，文字区在上、工具行在下。
  // 按键判断见 keys.ts（源自 Codeg）；输入器本身是 Kotone 的纯文本实现（不引入 Tiptap）。
  import type { Snippet } from "svelte";
  import { ArrowUp, Square } from "@lucide/svelte";
  import { shouldSubmitOnEnter } from "./keys";

  let {
    value = $bindable(""),
    disabled = false,
    blockedReason = "",
    running = false,
    stopping = false,
    onsubmit,
    onstop,
    tools,
    textarea = $bindable(),
    autofocus = false,
  }: {
    value?: string;
    /** 整个输入框不可用（例如语音输入进行中）。 */
    disabled?: boolean;
    /** 不能发送的原因（仍可编辑）。 */
    blockedReason?: string;
    running?: boolean;
    stopping?: boolean;
    /** 发送；返回 true 表示已交出去（清空输入框）。 */
    onsubmit: (text: string) => Promise<boolean>;
    onstop: () => void;
    /** 工具行左侧（语音输入）。 */
    tools?: Snippet;
    textarea?: HTMLTextAreaElement;
    autofocus?: boolean;
  } = $props();

  let composing = false;
  let sending = $state(false);
  const canSend = $derived(!disabled && !sending && !blockedReason && value.trim().length > 0);

  async function submit() {
    if (!canSend) return;
    sending = true;
    const text = value.trim();
    try {
      if (await onsubmit(text)) { if (value.trim() === text) value = ""; }
    } finally {
      sending = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (!shouldSubmitOnEnter(event, composing)) return;
    event.preventDefault();
    void submit();
  }

  $effect(() => { if (autofocus) textarea?.focus(); });
</script>

<div class="rounded-[22px] border border-hairline bg-composer px-3 pt-2.5 pb-2 shadow-[0_10px_40px_-12px_rgb(0_0_0/0.45)] transition-[border-color,box-shadow] focus-within:border-primary/45 focus-within:shadow-[0_10px_40px_-12px_rgb(0_0_0/0.45),0_0_0_3px_color-mix(in_oklab,var(--color-primary)_14%,transparent)]" data-testid="agent-composer">
  <textarea
    bind:this={textarea}
    bind:value
    data-testid="assistant-request"
    aria-label="跟琴音说"
    rows="1"
    maxlength="4000"
    placeholder={running ? "琴音回答时也可以继续说（插话）…" : "跟琴音说点什么…"}
    {disabled}
    class="block max-h-[336px] min-h-12 w-full resize-none bg-transparent px-1.5 py-1 text-sm leading-6 text-foreground outline-none [field-sizing:content] placeholder:text-muted-foreground/80 disabled:cursor-not-allowed disabled:opacity-60"
    onkeydown={keydown}
    oncompositionstart={() => { composing = true; }}
    oncompositionend={() => { composing = false; }}
  ></textarea>
  <div class="mt-1 flex items-end justify-between gap-2">
    <div class="min-w-0 flex-1">{@render tools?.()}</div>
    <div class="flex shrink-0 items-center gap-1.5 pb-0.5">
      {#if running}
        <button type="button" data-testid="agent-stop" disabled={stopping} onclick={onstop}
          class="flex h-8 items-center gap-1.5 rounded-full border border-hairline px-3 text-xs font-medium text-foreground transition-colors hover:bg-muted disabled:opacity-50">
          <Square class="size-3 fill-current" />{stopping ? "停止中…" : "停止"}
        </button>
      {/if}
      <button type="button" data-testid="agent-send" disabled={!canSend} title={blockedReason || "Enter 发送，Shift+Enter 换行"} onclick={() => void submit()}
        class="flex h-8 items-center gap-1 rounded-full bg-primary pr-3.5 pl-2.5 text-xs font-semibold text-primary-foreground shadow-[0_0_16px_-4px_var(--color-primary)] transition-[opacity,filter] hover:brightness-110 disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none">
        <ArrowUp class="size-4" />{running ? "插话" : "发送"}
      </button>
    </div>
  </div>
</div>
