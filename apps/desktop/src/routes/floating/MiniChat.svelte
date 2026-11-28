<script lang="ts">
  // 聊天小窗（ADR 043 D6，2026-11-26 晚修订）：玩家：“收到了推送的事件，右下角展示小窗，本质就是一个小的chat界面呀”。
  // 就是聊天页：同一个对话状态（AgentChat）、同一个时间线（buildTurns）、同一套消息组件（ChatTurn 的紧凑样子，推送是
  // PushItem、卡片是 PluginCard、工具是 ToolCallCard、她说的话是一句带喇叭的），这段对话最近的几轮，新的在下面。
  // 先画 `from` 往后的几轮（Floating.svelte 定：这一次来的 + 前面几轮，最多 20 轮）；滚到顶再往前加载 PAGE 轮，滚动位置
  // 不跳（照聊天页 ChatTranscript：scrollAnchor.ts 的锚点和“到顶”的门槛）。往前 / 往后翻的按钮去掉了：rc.10 真机和 rc.14
  // 试玩里滚轮在不可聚焦的小窗里收得到（玩家：“小窗一个滚动”）。
  // 高度跟着“这一次来的”那几轮走（从 `anchor` 那一轮到最下面），最高 `maxList`。跟着最新的：`target` 那一轮放进来（比框矮就
  // 贴着最下面，比框高就从它上面开始看）；玩家自己往回翻了就不跟，翻回最下面又跟上，弹出了新的那条再跟上（照 dsh-orb-cordis
  // 的 followBottom，MIT）。
  // 卡片里的 App 不在 iframe 里滚：最高到列表能放下的高度（FLOATING_CARD_MAX，McpAppFrame 读），再高的底部渐隐、点“在 Kotone
  // 里看完整的”。滚轮落在卡片上时滚的是这个列表（iframe 自己不能滚，Chromium 把滚轮交给外面）。
  import { setContext, tick } from "svelte";
  import { SquareArrowOutUpRight, X } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { FLOATING_PLUGIN_APP } from "$lib/pluginUi";
  import ChatTurn from "../../lib/agentChat/ChatTurn.svelte";
  import type { AgentChat } from "../../lib/agentChat/chatController.svelte";
  import { captureScrollAnchor, crossedHistoryThreshold, restoreScrollAnchor } from "../../lib/agentChat/scrollAnchor";
  import type { Turn } from "../../lib/agentChat/timeline";
  import type { PushRecord } from "../../lib/agent";
  import type { BubbleSnapshot } from "../../lib/bubble";
  import { FLOATING_CARD_MAX, type FloatingCardMax } from "../../lib/floating";
  import HerLine from "./HerLine.svelte";
  import QuickComposer from "./QuickComposer.svelte";
  import { turnPushId } from "./shape";

  let { turns, from = 0, anchor, target, pop, maxList, width = 360, title, chat, status, onOpen, onOpenPush, onClose, onDrag, onLayout,
    active = true, conversationId = null, onInputFocus = () => {}, onComposerActive = () => {} }: {
    /** 整段对话的时间线。 */
    turns: Turn[];
    /** 先从第几轮画起（更早的滚到顶再加载）。 */
    from?: number;
    /** 高度从这一轮算起（这一次来的第一轮）。 */
    anchor: string | null;
    /** 跟着时放进来的那一轮。 */
    target: string | null;
    /** 弹出的那条推送（换了就再跟上）。 */
    pop: string | null;
    maxList: number;
    /** 小窗多宽（px，设置里的“小窗宽度”，后端夹过）。 */
    width?: number;
    title: string;
    chat: AgentChat;
    /** 她这时候的状态行（在听、等你回答……），对话里还没有的。 */
    status: BubbleSnapshot | null;
    /** 打开 Kotone、定位到这一条推送（空 = 这段对话）。 */
    onOpen: (uri?: string) => void;
    onOpenPush: (push: PushRecord) => void;
    onClose: () => void;
    onDrag: (event: PointerEvent) => void;
    /** 高度、滚动变了：外面再量一次报给后端。 */
    onLayout: () => void;
    active?: boolean;
    conversationId?: string | null;
    onInputFocus?: (focused: boolean) => void | Promise<void>;
    onComposerActive?: (active: boolean) => void;
  } = $props();
  setContext(FLOATING_PLUGIN_APP, true);

  /** 列表最矮多高（卡片还没画出来时）。 */
  const MIN_LIST = 72;
  /** 列表上下的留白（px，和样子里的 padding 一样）。 */
  const TOP_PADDING = 4;
  const PADDING = TOP_PADDING + 8;
  /** 滚到顶一次往前加载几轮（和聊天页一样）。 */
  const PAGE = 20;
  /** 卡片上面那几行（推送的“英雄联盟 推送”一行、卡片头、上下的留白）：卡片里的 App 最高 = 列表能放下的减掉它们。 */
  const CARD_CHROME = 64;
  /** 卡片里的 App 最矮也给这么高（列表很矮时照样能看到一截，渐隐、点开看完整的）。 */
  const CARD_MIN = 120;

  let list = $state<HTMLDivElement>();
  let inner = $state<HTMLDivElement>();
  let composerHeight = $state(0);
  let height = $state(MIN_LIST);
  let follow = true;
  /** 自己滚的那一下引起的 scroll 不算玩家在翻。 */
  let ownScrollUntil = 0;
  let seenPop: string | null = null;
  /** 滚到顶以后又往前加载了几轮（换了对话从头算）。 */
  let more = $state(0);
  let seenConversation: string | null | undefined;
  let previousTop: number | null = null;

  const start = $derived(Math.max(0, Math.min(from, turns.length) - more));
  const visible = $derived(turns.slice(start));
  // 卡片里的 App 最高多高（McpAppFrame 读；跟着列表最高能多高走）。卡片截出来的高度一变就重新量列表（resized）：列表的
  // 高度跟着内容，卡片长高的那一轮里列表也跟着长，不会有一下卡片比列表高（列表到顶时卡片最高正好是列表放得下的）
  const cardMax: FloatingCardMax = {
    get max() { return Math.max(CARD_MIN, maxList - composerHeight - PADDING - CARD_CHROME); },
    resized: () => void relayout(),
  };
  setContext(FLOATING_CARD_MAX, cardMax);

  $effect.pre(() => {
    if (seenConversation !== undefined && conversationId !== seenConversation) {
      more = 0;
      previousTop = null;
    }
    seenConversation = conversationId;
  });

  /** 一轮在列表里（滚动坐标）的上沿。 */
  function topOf(id: string | null): number | null {
    if (!id || !list) return null;
    const el = list.querySelector<HTMLElement>(`[data-floating-turn="${CSS.escape(id)}"]`);
    if (!el) return null;
    return el.offsetTop + TOP_PADDING;
  }

  let queued = false;
  async function relayout() {
    if (queued) return;
    queued = true;
    await tick();
    queued = false;
    if (!list || !inner) return;
    // 从那一轮的上沿到内容最下面（量里面那一块：列表比内容高时 scrollHeight 不会变小），加上列表上下的留白
    const el = anchor ? inner.querySelector<HTMLElement>(`[data-floating-turn="${CSS.escape(anchor)}"]`) : null;
    const from = el?.offsetTop ?? 0;
    height = Math.max(MIN_LIST, Math.min(inner.offsetHeight - from + PADDING, maxList - composerHeight));
    await tick();
    if (follow && list) {
      ownScrollUntil = performance.now() + 200;
      const bottom = list.scrollHeight - list.clientHeight;
      const top = topOf(target);
      list.scrollTop = top === null ? bottom : Math.min(Math.max(0, top - TOP_PADDING), bottom);
    }
    if (list) previousTop = list.scrollTop;
    onLayout();
  }

  function onScroll() {
    if (!list) return;
    const top = list.scrollTop;
    const crossed = crossedHistoryThreshold(previousTop, top);
    const reachedTop = previousTop !== null && previousTop > top && top <= 2;
    previousTop = top;
    // 滚到顶（或快到顶）：往前再加载一页（小窗矮，滚动范围可能不到聊天页的门槛，到了顶也算）。自己滚的那几下设完
    // scrollTop 马上记下 previousTop，不会算成“往上滚过了门槛”，所以这里不看 ownScrollUntil（刚加载完紧接着再往上滚也接得上）
    if ((crossed || reachedTop) && start > 0) void revealOlder();
    if (performance.now() < ownScrollUntil) return;
    // 玩家往回翻了就不跟；翻回最下面又跟上
    follow = top + list.clientHeight >= list.scrollHeight - 24;
  }

  /** 往前加载 PAGE 轮，玩家正在看的那一轮留在原处（scrollAnchor.ts）。 */
  async function revealOlder() {
    if (!list || start <= 0) return;
    const element = list;
    const kept = captureScrollAnchor(element);
    follow = false;
    more += PAGE;
    await tick();
    ownScrollUntil = performance.now() + 200;
    if (kept) restoreScrollAnchor(element, kept);
    previousTop = element.scrollTop;
  }

  // 新的一轮、原地更新、她的状态、弹出了新的那条：再量一次
  $effect(() => {
    void visible; void anchor; void target; void status; void maxList; void composerHeight;
    if (pop !== seenPop) { seenPop = pop; follow = true; }
    void relayout();
  });

  // 内容变大变小（卡片里的 App 画出来、她的话变长）：再量一次
  $effect(() => {
    if (!inner || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => void relayout());
    observer.observe(inner);
    return () => observer.disconnect();
  });
</script>

<section class="mini ui-scope" style:width="{width}px" data-testid="floating-mini" data-focus={pop ?? ""} aria-label={m.chat_floating_mini_label()}>
  <div class="head" role="presentation" onpointerdown={onDrag} data-testid="floating-mini-head">
    <span class="title">{title}</span>
    <button type="button" class="tool" aria-label={m.chat_floating_open()} title={m.chat_floating_open()} data-testid="floating-open"
      onclick={() => onOpen()}><SquareArrowOutUpRight class="size-3.5" /></button>
    <button type="button" class="tool" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
      onclick={onClose}><X class="size-3.5" /></button>
  </div>
  <div bind:this={list} class="list" style:height="{height}px" onscroll={onScroll} data-testid="floating-list"
    data-start={start} data-total={turns.length}>
    <div bind:this={inner} class="inner">
      {#each visible as turn (turn.id)}
        <div class="turn" data-floating-turn={turn.id} data-testid="floating-turn" data-push={turnPushId(turn) ?? ""}>
          <ChatTurn {turn} compact askFor={(callId) => chat.askFor(callId)} onAnswer={(askId, answer) => chat.answer(askId, answer)}
            {onOpenPush} onOpenCard={onOpen} />
        </div>
      {/each}
      {#if status}
        <HerLine snap={status} compact />
      {/if}
    </div>
  </div>
  <div bind:offsetHeight={composerHeight} class="composer">
    <QuickComposer {active} {chat} {conversationId} name={title} {onInputFocus} onActiveChange={onComposerActive} />
  </div>
</section>

<style>
  .mini {
    --floating-surface: var(--card);
    --floating-raised: var(--muted);
    --floating-line: var(--hairline, var(--border));
    --floating-ink: var(--foreground);
    --floating-muted: var(--muted-foreground);
    display: flex; flex-direction: column; overflow: hidden; box-sizing: border-box;
    background: var(--floating-surface); color: var(--floating-ink); border: 1px solid var(--floating-line);
    border-radius: 12px; box-shadow: var(--floating-shadow);
  }
  :global(:root[data-theme="dark"]) .mini {
    --floating-surface: #171916;
    --floating-raised: #22251f;
    --floating-line: #343832;
    --floating-ink: #eeefe6;
    --floating-muted: #acb0a6;
    --card: var(--floating-surface);
    --foreground: var(--floating-ink);
    --muted: var(--floating-raised);
    --muted-foreground: var(--floating-muted);
    --hairline: var(--floating-line);
  }
  .head {
    display: flex; height: 32px; flex: none; align-items: center; gap: 2px; padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--floating-line); cursor: grab;
  }
  .title { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; font-weight: 600; color: var(--floating-ink); }
  .tool {
    display: flex; width: 24px; height: 24px; align-items: center; justify-content: center; border: 0; border-radius: 6px;
    background: transparent; color: var(--muted-foreground); cursor: pointer;
  }
  .tool:hover:not(:disabled) { background: var(--muted); color: var(--foreground); }
  .tool:disabled { opacity: .35; cursor: default; }
  .list { box-sizing: border-box; overflow-y: auto; overscroll-behavior: contain; padding: 4px 10px 8px; scrollbar-width: thin; }
  .inner { position: relative; display: flex; flex-direction: column; gap: 14px; }
  .turn { min-width: 0; }
  .composer {
    flex: none;
    padding: 0 calc(10px + var(--floating-compose-end, 0px)) 9px calc(10px + var(--floating-compose-start, 0px));
  }
</style>
