<script lang="ts">
  // 一轮对话：用户气泡（或自动化结果的系统行）+ 琴音的思考、回答、工具行、状态说明 + 页脚。
  // 版式参照 DeepSeek Harness 的消息流（MessageItem / UserMessageBubble，MIT）：用户消息是右侧
  // 圆角气泡，回答不加气泡，过程是一行一行的“流程行”。Kotone 自己的实现。
  // 悬浮窗的聊天小窗（ADR 043 D6）也画它：`compact` 去掉每一轮上面她的小头像和名字（悬浮窗角上就是她的头像）和下面的
  // 页脚（复制、时间、用量），间距紧一点；小窗里没有右侧栏，卡片上面那一行交给 `onOpenPush` / `onOpenCard`（打开 Kotone）。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
  import { ChevronRight, Info, Volume2, Zap } from "@lucide/svelte";
  import { movedTaskName, sourceLabel, type AgentAsk, type Answer, type PushRecord } from "../agent";
  import MessageMarkdown from "./markdown/MessageMarkdown.svelte";
  import ThinkingBlock from "./ThinkingBlock.svelte";
  import ToolCallCard from "./ToolCallCard.svelte";
  import BackgroundTaskCard from "./BackgroundTaskCard.svelte";
  import PushItem from "./PushItem.svelte";
  import TurnFooter from "./TurnFooter.svelte";
  import type { Turn } from "./timeline";
  import { formatToolJson } from "./toolPresentation";
  import KotoneAvatar from "../components/KotoneAvatar.svelte";
  import { avatarLetter, customAvatar, roleFaces } from "../stores/avatar";
  import { settingsStore } from "../stores/ui";

  let { turn, askFor, onAnswer, compact = false, onOpenPush, onOpenCard }: {
    turn: Turn;
    askFor: (callId: string) => AgentAsk | undefined;
    onAnswer: (askId: string, answer: Answer) => Promise<void>;
    /** 悬浮窗的聊天小窗：不画每一轮上面的小头像和名字、不画页脚，间距紧一点。 */
    compact?: boolean;
    /** 给了就交给它（悬浮窗：打开 Kotone、定位到这条推送），不在右侧栏打开。 */
    onOpenPush?: (push: PushRecord) => void;
    /** 工具结果的卡片：给了就交给它（悬浮窗：打开 Kotone）。 */
    onOpenCard?: () => void;
  } = $props();

  // 后台做完的事交回来的那条（ADR 029）：卡片已经更新，不显示这条，只显示她接着说的话。
  const user = $derived(turn.user?.source === "agent-report" ? null : turn.user);
  const system = $derived(user?.source === "automation");
  const source = $derived(sourceLabel(user?.source));
  // i18n-ignore-start：下面两处匹配的是后端写的字（自动化结果消息、speak 的结果“玩家打断了…”），不是界面文字
  /** 自动化结果：后端写给模型的说明里取出“自动化「…」已完成”这一句给人看。 */
  const systemText = $derived(user?.text.match(/自动化「[^」]*」[^。；）]*/)?.[0] ?? user?.text ?? "");
  const INTERRUPTED_MARK = "打断";
  // i18n-ignore-end
  const waiting = $derived(turn.active && !turn.blocks.some((b) => b.kind === "text" && b.streaming));

  /** `speak` 的参数（ADR 029 第八节）：text 念出来，written 只显示在聊天里。参数还没写完时为空。 */
  function speakArgs(args: string): { text: string; written: string } {
    try {
      const value = JSON.parse(args);
      return { text: String(value?.text ?? ""), written: String(value?.written ?? "") };
    } catch { return { text: "", written: "" }; }
  }
</script>

<section class={compact ? "space-y-2" : "space-y-3"} data-turn-anchor={turn.id} data-testid="agent-turn" data-turn-active={turn.active}>
  {#if turn.push}
    {@const push = turn.push}
    <!-- 推送（ADR 043 D3）：插件自己发来的，单独一轮 -->
    <PushItem {push} atMs={turn.endedAtMs ?? 0} onOpen={onOpenPush ? () => onOpenPush(push) : undefined} />
  {/if}
  {#if user && system}
    <details class="group/system min-w-0 text-[13px] text-muted-foreground" data-testid="agent-system-message">
      <summary class="flex min-h-7 w-fit max-w-full cursor-pointer list-none items-center gap-2 rounded-md pr-1.5 hover:text-foreground">
        <Zap class="size-3.5 shrink-0 text-info" />
        <span class="shrink-0 text-info">{m.chat_source_automation()}</span>
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
        <span class="text-[11px] text-muted-foreground">{m.chat_turn_attachment({ label: attachment.label })}</span>
      {/each}
    </div>
  {/if}

  {#if turn.blocks.length || turn.active}
    <div class="min-w-0 space-y-1.5">
      {#if !compact}
        <div class="flex items-center gap-2 pb-0.5 text-xs text-muted-foreground" aria-hidden="true">
          <KotoneAvatar size={20} mood="speaking" custom={$customAvatar} faces={$roleFaces} letter={$avatarLetter} animate={turn.active} />
          <span class="font-medium text-foreground/80">{$companionName}</span>
        </div>
      {/if}
      {#each turn.blocks as block (block.key)}
        {#if block.kind === "thinking"}
          <ThinkingBlock text={block.text} streaming={block.streaming} />
        {:else if block.kind === "text"}
          <div class="py-0.5" data-testid="agent-answer" data-streaming={block.streaming}><MessageMarkdown text={block.text} /></div>
        {:else if block.kind === "tool" && block.tool.call.name === "speak"}
          {@const said = speakArgs(block.tool.call.arguments)}
          <!-- 她说出口的话：显示成一句话，不是工具卡片；只写不念的细节在下面 -->
          <p class="m-0 flex items-start gap-2 py-0.5 text-sm leading-[22px] text-foreground" data-testid="agent-spoken">
            <Volume2 aria-label={m.chat_turn_spoken_label()} class="mt-[3px] size-3.5 shrink-0 text-primary" />
            <span class="min-w-0 break-words">{m.chat_turn_spoken_quote({ text: said.text })}{#if block.tool.content?.includes(INTERRUPTED_MARK)}<span class="ml-1.5 text-xs text-muted-foreground" data-testid="agent-spoken-interrupted">{m.chat_turn_spoken_interrupted()}</span>{/if}</span>
          </p>
          {#if said.written.trim()}
            <div class="py-0.5" data-testid="agent-written"><MessageMarkdown text={said.written} /></div>
          {/if}
        {:else if block.kind === "tool" && (block.tool.call.name === "agent.start" || block.tool.call.name === "agent.followup" || movedTaskName(block.tool))}
          <!-- 交给后台的事；主会话里超过 3 秒转到后台的工具也显示成同一种卡片（ADR 029 §一） -->
          <BackgroundTaskCard tool={block.tool} runActive={turn.active} />
        {:else if block.kind === "tool"}
          <ToolCallCard tool={block.tool} runActive={turn.active} ask={askFor(block.tool.call.id)} {onAnswer} {onOpenCard} />
        {:else if block.kind === "recap"}
          <div class="my-1 rounded-xl border border-primary/30 bg-primary/5 px-3 py-2" data-testid="agent-recap">
            <p class="m-0 flex items-center gap-2 text-[13px] font-medium text-foreground"><Info class="size-3.5 shrink-0 text-primary" />{block.text}</p>
            <p class="m-0 mt-1.5 whitespace-pre-wrap text-[13px] leading-relaxed text-foreground/85">{block.body}</p>
          </div>
        {:else if block.kind === "checkpoint"}
          <details class="group my-1 rounded-lg border border-dashed border-border/80 px-3 py-1.5 text-[13px] text-muted-foreground" data-testid="agent-checkpoint">
            <summary class="flex cursor-pointer list-none items-center gap-2"><Info class="size-3.5 shrink-0" />{block.text}<span class="ml-auto text-xs text-primary group-open:hidden">{m.chat_turn_checkpoint_show_summary()}</span></summary>
            <pre class="m-0 mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words font-sans text-xs leading-relaxed text-foreground/80">{block.summary}</pre>
          </details>
        {:else}
          <p class="m-0 flex min-h-7 items-center gap-2 text-[13px] text-muted-foreground" data-testid="agent-notice"><Info class="size-3.5 shrink-0" />{block.text}</p>
        {/if}
      {/each}
      {#if waiting}
        <p class="m-0 flex h-7 items-center text-[13px] text-muted-foreground" role="status"><span class="kotone-shimmer">{m.chat_turn_thinking({ name: $companionName })}</span></p>
      {/if}
      {#if !turn.active && !compact}<TurnFooter copyText={turn.answerText} endedAtMs={turn.endedAtMs} usage={turn.usage} extraCosts={turn.extraCosts ?? []} />{/if}
    </div>
  {/if}
</section>
