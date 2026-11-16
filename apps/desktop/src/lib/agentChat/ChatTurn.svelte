<script lang="ts">
  // 一轮对话：用户气泡（或自动化结果的系统卡片）+ 思考、回答、工具卡片、状态说明 + 页脚。
  // 布局参照 RambleDesk AgentTurn.svelte（源自 Codeg CompletedTurnContent），这里不折叠过程。
  import { Bot, Info, LoaderCircle } from "@lucide/svelte";
  import { sourceLabel, type AgentAsk, type Answer } from "../agent";
  import MessageMarkdown from "./markdown/MessageMarkdown.svelte";
  import ThinkingBlock from "./ThinkingBlock.svelte";
  import ToolCallCard from "./ToolCallCard.svelte";
  import TurnFooter from "./TurnFooter.svelte";
  import type { Turn } from "./timeline";
  import { formatToolJson } from "./toolPresentation";

  let { turn, askFor, onAnswer }: {
    turn: Turn;
    askFor: (callId: string) => AgentAsk | undefined;
    onAnswer: (askId: string, answer: Answer) => Promise<void>;
  } = $props();

  const user = $derived(turn.user);
  const system = $derived(user?.source === "automation");
  const source = $derived(sourceLabel(user?.source));
  /** 自动化结果：后端写给模型的说明里取出“自动化「…」已完成”这一句给人看。 */
  const systemText = $derived(user?.text.match(/自动化「[^」]*」[^。；）]*/)?.[0] ?? user?.text ?? "");
  const waiting = $derived(turn.active && !turn.blocks.some((b) => b.kind === "text" && b.streaming));
</script>

<section class="space-y-2.5" data-turn-anchor={turn.id} data-testid="agent-turn" data-turn-active={turn.active}>
  {#if user && system}
    <div class="flex items-start gap-2 rounded-lg border border-info/30 bg-info/5 px-3 py-2 text-xs" data-testid="agent-system-message">
      <Info class="mt-0.5 size-3.5 shrink-0 text-info" />
      <div class="min-w-0">
        <p class="m-0 font-medium text-info">自动化结果</p>
        <p class="m-0 mt-0.5 break-words whitespace-pre-wrap text-foreground/85">{systemText}</p>
        {#each user.attachments ?? [] as attachment, i (i)}
          {#if attachment.kind === "data"}
            <details class="mt-1 text-muted-foreground">
              <summary class="cursor-pointer text-[11px] hover:text-foreground">{attachment.label}</summary>
              <pre class="mt-1 mb-0 max-h-48 overflow-auto font-mono text-[11px] leading-5 break-words whitespace-pre-wrap">{formatToolJson(attachment.text)}</pre>
            </details>
          {/if}
        {/each}
      </div>
    </div>
  {:else if user}
    <div class="flex flex-col items-end gap-1">
      {#if source}<span class="text-[10px] text-muted-foreground" data-testid="agent-user-source">{source}</span>{/if}
      <div class="max-w-[85%] rounded-2xl rounded-br-md bg-primary/15 px-3.5 py-2 text-sm break-words whitespace-pre-wrap text-foreground ring-1 ring-primary/25" data-testid="agent-user-message">{user.text}</div>
      {#each user.attachments ?? [] as attachment, i (i)}
        <span class="text-[10px] text-muted-foreground">附带：{attachment.label}</span>
      {/each}
    </div>
  {/if}

  {#if turn.blocks.length || turn.active}
    <div class="flex items-start gap-2.5">
      <span class="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary" aria-hidden="true"><Bot class="size-3.5" /></span>
      <div class="min-w-0 flex-1 space-y-2">
        {#each turn.blocks as block (block.key)}
          {#if block.kind === "thinking"}
            <ThinkingBlock text={block.text} streaming={block.streaming} />
          {:else if block.kind === "text"}
            <div data-testid="agent-answer" data-streaming={block.streaming}><MessageMarkdown text={block.text} /></div>
          {:else if block.kind === "tool"}
            <ToolCallCard tool={block.tool} runActive={turn.active} ask={askFor(block.tool.call.id)} {onAnswer} />
          {:else}
            <p class="m-0 text-xs text-muted-foreground" data-testid="agent-notice">{block.text}</p>
          {/if}
        {/each}
        {#if waiting}
          <p class="m-0 flex items-center gap-1.5 text-xs text-muted-foreground" role="status"><LoaderCircle class="size-3.5 animate-spin" />琴音在想…</p>
        {/if}
        {#if !turn.active}<TurnFooter copyText={turn.answerText} endedAtMs={turn.endedAtMs} usage={turn.usage} />{/if}
      </div>
    </div>
  {/if}
</section>
