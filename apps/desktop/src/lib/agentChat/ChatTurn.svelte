<script lang="ts">
  // 一轮对话：用户气泡（或自动化结果的系统行）+ 琴音的思考、回答、工具行、状态说明 + 页脚。
  // 版式参照 DeepSeek Harness 的消息流（MessageItem / UserMessageBubble，MIT）：用户消息是右侧
  // 圆角气泡，回答不加气泡，过程是一行一行的“流程行”。Kotone 自己的实现。
  import { ChevronRight, Info, Zap } from "@lucide/svelte";
  import { sourceLabel, type AgentAsk, type Answer } from "../agent";
  import MessageMarkdown from "./markdown/MessageMarkdown.svelte";
  import ThinkingBlock from "./ThinkingBlock.svelte";
  import ToolCallCard from "./ToolCallCard.svelte";
  import TurnFooter from "./TurnFooter.svelte";
  import type { Turn } from "./timeline";
  import { formatToolJson } from "./toolPresentation";
  import KotoneAvatar from "../components/KotoneAvatar.svelte";
  import { avatarLetter, customAvatar } from "../stores/avatar";
  import { settingsStore } from "../stores/ui";

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

<section class="space-y-3" data-turn-anchor={turn.id} data-testid="agent-turn" data-turn-active={turn.active}>
  {#if user && system}
    <details class="group/system min-w-0 text-[13px] text-muted-foreground" data-testid="agent-system-message">
      <summary class="flex min-h-7 w-fit max-w-full cursor-pointer list-none items-center gap-2 rounded-md pr-1.5 hover:text-foreground">
        <Zap class="size-3.5 shrink-0 text-info" />
        <span class="shrink-0 text-info">自动化结果</span>
        <span class="size-0.5 shrink-0 rounded-full bg-muted-foreground/60" aria-hidden="true"></span>
        <span class="min-w-0 truncate text-foreground/85">{systemText}</span>
        {#if user.attachments?.length}<ChevronRight aria-hidden="true" class="size-3.5 shrink-0 opacity-0 transition-[opacity,transform] group-hover/system:opacity-100 group-open/system:rotate-90 group-open/system:opacity-100" />{/if}
      </summary>
      {#each user.attachments ?? [] as attachment, i (i)}
        {#if attachment.kind === "data"}
          <div class="mt-1 mb-2 ml-1 rounded-xl border border-hairline bg-muted/40 px-4 py-3">
            <p class="m-0 mb-1 text-[11px] font-medium">{attachment.label}</p>
            <pre class="m-0 max-h-60 overflow-auto font-mono text-xs leading-5 break-words whitespace-pre-wrap text-foreground/90">{formatToolJson(attachment.text)}</pre>
          </div>
        {/if}
      {/each}
    </details>
  {:else if user}
    <div class="flex flex-col items-end gap-1.5">
      {#if source}<span class="text-[11px] text-muted-foreground" data-testid="agent-user-source">{source}</span>{/if}
      <div class="max-w-[min(82%,34rem)] rounded-[20px] bg-bubble px-4 py-2.5 text-sm leading-[22px] break-words whitespace-pre-wrap text-foreground" data-testid="agent-user-message">{user.text}</div>
      {#each user.attachments ?? [] as attachment, i (i)}
        <span class="text-[11px] text-muted-foreground">附带：{attachment.label}</span>
      {/each}
    </div>
  {/if}

  {#if turn.blocks.length || turn.active}
    <div class="min-w-0 space-y-1.5">
      <div class="flex items-center gap-2 pb-0.5 text-xs text-muted-foreground" aria-hidden="true">
        <KotoneAvatar size={20} mood="speaking" custom={$customAvatar} letter={$avatarLetter} animate={turn.active} />
        <span class="font-medium text-foreground/80">{$settingsStore?.agent?.identity?.name?.trim() || "琴音"}</span>
      </div>
      {#each turn.blocks as block (block.key)}
        {#if block.kind === "thinking"}
          <ThinkingBlock text={block.text} streaming={block.streaming} />
        {:else if block.kind === "text"}
          <div class="py-0.5" data-testid="agent-answer" data-streaming={block.streaming}><MessageMarkdown text={block.text} /></div>
        {:else if block.kind === "tool"}
          <ToolCallCard tool={block.tool} runActive={turn.active} ask={askFor(block.tool.call.id)} {onAnswer} />
        {:else if block.kind === "checkpoint"}
          <details class="group my-1 rounded-lg border border-dashed border-border/80 px-3 py-1.5 text-[13px] text-muted-foreground" data-testid="agent-checkpoint">
            <summary class="flex cursor-pointer list-none items-center gap-2"><Info class="size-3.5 shrink-0" />{block.text}<span class="ml-auto text-xs text-primary group-open:hidden">看摘要</span></summary>
            <pre class="m-0 mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words font-sans text-xs leading-relaxed text-foreground/80">{block.summary}</pre>
          </details>
        {:else}
          <p class="m-0 flex min-h-7 items-center gap-2 text-[13px] text-muted-foreground" data-testid="agent-notice"><Info class="size-3.5 shrink-0" />{block.text}</p>
        {/if}
      {/each}
      {#if waiting}
        <p class="m-0 flex h-7 items-center text-[13px] text-muted-foreground" role="status"><span class="kotone-shimmer">琴音在想…</span></p>
      {/if}
      {#if !turn.active}<TurnFooter copyText={turn.answerText} endedAtMs={turn.endedAtMs} usage={turn.usage} />{/if}
    </div>
  {/if}
</section>
