<script lang="ts">
  // 输入框：Enter 发送、Shift+Enter 换行、输入法组字时不发送；回答中显示“停止”，仍可发送（插话）。
  // 按键判断见 keys.ts（源自 Codeg）；输入器本身是 Kotone 的纯文本实现（不引入 Tiptap）。
  import type { Snippet } from "svelte";
  import { ArrowUp, Square } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
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
    /** 输入框下方左侧的工具（语音输入）。 */
    tools?: Snippet;
    textarea?: HTMLTextAreaElement;
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
</script>

<div class="rounded-xl border bg-card/70 p-2 shadow-sm focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30" data-testid="agent-composer">
  <textarea
    bind:this={textarea}
    bind:value
    data-testid="assistant-request"
    aria-label="跟琴音说"
    rows="2"
    maxlength="4000"
    placeholder={running ? "琴音回答时也可以继续说（插话）…" : "跟琴音说点什么…  Enter 发送，Shift+Enter 换行"}
    {disabled}
    class="block max-h-48 min-h-11 w-full resize-none bg-transparent px-1.5 py-1 text-sm leading-6 text-foreground outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-60 [field-sizing:content]"
    onkeydown={keydown}
    oncompositionstart={() => { composing = true; }}
    oncompositionend={() => { composing = false; }}
  ></textarea>
  <div class="mt-1 flex items-end justify-between gap-2">
    <div class="min-w-0 flex-1">{@render tools?.()}</div>
    <div class="flex shrink-0 items-center gap-1.5">
      {#if running}
        <Button variant="outline" size="sm" data-testid="agent-stop" disabled={stopping} onclick={onstop}>
          <Square class="size-3 fill-current" />{stopping ? "停止中…" : "停止"}
        </Button>
      {/if}
      <Button size="sm" data-testid="agent-send" disabled={!canSend} title={blockedReason || undefined} onclick={() => void submit()}>
        <ArrowUp class="size-3.5" />{running ? "插话" : "发送"}
      </Button>
    </div>
  </div>
</div>
