<script lang="ts">
  // 聊天小窗（ADR 043 D6，2026-11-26 晚修订）：玩家：“收到了推送的事件，右下角展示小窗，本质就是一个小的chat界面呀”。
  // 就是聊天页：同一个对话状态（AgentChat）、同一个时间线（buildTurns）、同一套消息组件（ChatTurn 的紧凑样子，推送是
  // PushItem、卡片是 PluginCard、工具是 ToolCallCard、她说的话是一句带喇叭的），这段对话最近的几轮，新的在下面。
  // 高度跟着“这一次来的”那几轮走（从 `anchor` 那一轮到最下面），最高 `maxList`；更早的往回翻（滚轮；滚轮在不可聚焦的窗口里
  // 真机验证前先留着往前 / 往后翻的按钮，能翻时才出来）。跟着最新的：`target` 那一轮放进来（比框矮就贴着最下面，比框高就
  // 从它上面开始看）；玩家自己往回翻了就不跟，翻回最下面又跟上，弹出了新的那条再跟上（照 dsh-orb-cordis 的 followBottom，MIT）。
  import { tick } from "svelte";
  import { ChevronDown, ChevronUp, SquareArrowOutUpRight, X } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import ChatTurn from "../../lib/agentChat/ChatTurn.svelte";
  import type { AgentChat } from "../../lib/agentChat/chatController.svelte";
  import type { Turn } from "../../lib/agentChat/timeline";
  import type { PushRecord } from "../../lib/agent";
  import type { BubbleSnapshot } from "../../lib/bubble";
  import HerLine from "./HerLine.svelte";
  import { turnPushId } from "./shape";

  let { turns, anchor, target, pop, maxList, title, chat, status, onOpen, onOpenPush, onClose, onDrag, onLayout }: {
    turns: Turn[];
    /** 高度从这一轮算起（这一次来的第一轮）。 */
    anchor: string | null;
    /** 跟着时放进来的那一轮。 */
    target: string | null;
    /** 弹出的那条推送（换了就再跟上）。 */
    pop: string | null;
    maxList: number;
    title: string;
    chat: AgentChat;
    /** 她这时候的状态行（在听、等你回答……），对话里还没有的。 */
    status: BubbleSnapshot | null;
    /** 打开 Kotone、定位到这一条推送（空 = 这段对话）。 */
    onOpen: (key: string) => void;
    onOpenPush: (push: PushRecord) => void;
    onClose: () => void;
    onDrag: (event: PointerEvent) => void;
    /** 高度、滚动变了：外面再量一次报给后端。 */
    onLayout: () => void;
  } = $props();

  /** 列表最矮多高（卡片还没画出来时）。 */
  const MIN_LIST = 72;
  /** 列表上下的留白（px，和样子里的 padding 一样）。 */
  const PADDING = 4 + 8;

  let list = $state<HTMLDivElement>();
  let inner = $state<HTMLDivElement>();
  let height = $state(MIN_LIST);
  let follow = true;
  /** 自己滚的那一下引起的 scroll 不算玩家在翻。 */
  let ownScrollUntil = 0;
  let canUp = $state(false);
  let canDown = $state(false);
  let seenPop: string | null = null;

  /** 一轮在列表里（滚动坐标）的上沿。 */
  function topOf(id: string | null): number | null {
    if (!id || !list) return null;
    const el = list.querySelector<HTMLElement>(`[data-floating-turn="${CSS.escape(id)}"]`);
    if (!el) return null;
    return el.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop;
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
    const from = el ? el.getBoundingClientRect().top - inner.getBoundingClientRect().top : 0;
    height = Math.max(MIN_LIST, Math.min(Math.ceil(inner.offsetHeight - from) + PADDING, maxList));
    await tick();
    if (follow && list) {
      ownScrollUntil = performance.now() + 200;
      const bottom = list.scrollHeight - list.clientHeight;
      const top = topOf(target);
      list.scrollTop = top === null ? bottom : Math.min(Math.max(0, top - 4), bottom);
    }
    arrows();
    onLayout();
  }

  function arrows() {
    if (!list) return;
    canUp = list.scrollTop > 2;
    canDown = list.scrollTop + list.clientHeight < list.scrollHeight - 2;
  }

  function onScroll() {
    arrows();
    if (!list || performance.now() < ownScrollUntil) return;
    // 玩家往回翻了就不跟；翻回最下面又跟上
    follow = list.scrollTop + list.clientHeight >= list.scrollHeight - 24;
  }

  function page(direction: 1 | -1) {
    list?.scrollBy({ top: direction * Math.max(48, list.clientHeight * 0.8), behavior: "smooth" });
  }

  // 新的一轮、原地更新、她的状态、弹出了新的那条：再量一次
  $effect(() => {
    void turns; void anchor; void target; void status; void maxList;
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

<section class="mini ui-scope" data-testid="floating-mini" data-focus={pop ?? ""} aria-label={m.chat_floating_mini_label()}>
  <div class="head" role="presentation" onpointerdown={onDrag} data-testid="floating-mini-head">
    <span class="title">{title}</span>
    {#if canUp || canDown}
      <!-- 往回翻（滚轮之外的办法） -->
      <button type="button" class="tool" disabled={!canUp} aria-label={m.chat_floating_older()} title={m.chat_floating_older()}
        data-testid="floating-older" onclick={() => page(-1)}><ChevronUp class="size-3.5" /></button>
      <button type="button" class="tool" disabled={!canDown} aria-label={m.chat_floating_newer()} title={m.chat_floating_newer()}
        data-testid="floating-newer" onclick={() => page(1)}><ChevronDown class="size-3.5" /></button>
    {/if}
    <button type="button" class="tool" aria-label={m.chat_floating_open()} title={m.chat_floating_open()} data-testid="floating-open"
      onclick={() => onOpen("")}><SquareArrowOutUpRight class="size-3.5" /></button>
    <button type="button" class="tool" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
      onclick={onClose}><X class="size-3.5" /></button>
  </div>
  <div bind:this={list} class="list" style:height="{height}px" onscroll={onScroll} data-testid="floating-list">
    <div bind:this={inner} class="inner">
      {#each turns as turn (turn.id)}
        <div class="turn" data-floating-turn={turn.id} data-testid="floating-turn" data-push={turnPushId(turn) ?? ""}>
          <ChatTurn {turn} compact askFor={(callId) => chat.askFor(callId)} onAnswer={(askId, answer) => chat.answer(askId, answer)}
            {onOpenPush} onOpenCard={() => onOpen("")} />
        </div>
      {/each}
      {#if status}
        <HerLine snap={status} compact />
      {/if}
    </div>
  </div>
</section>

<style>
  .mini {
    display: flex; flex-direction: column; width: 360px; overflow: hidden; box-sizing: border-box;
    background: var(--card); color: var(--foreground); border: 1px solid var(--hairline, var(--border));
    border-radius: 12px; box-shadow: 0 8px 24px rgb(0 0 0 / 0.18);
    animation: enter 180ms cubic-bezier(.2, .8, .3, 1) both;
  }
  :global(:root[data-theme="dark"]) .mini { box-shadow: 0 8px 24px rgb(0 0 0 / 0.55); }
  .head {
    display: flex; height: 32px; flex: none; align-items: center; gap: 2px; padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--hairline, var(--border)); cursor: grab;
  }
  .title { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; font-weight: 600; color: var(--muted-foreground); }
  .tool {
    display: flex; width: 24px; height: 24px; align-items: center; justify-content: center; border: 0; border-radius: 6px;
    background: transparent; color: var(--muted-foreground); cursor: pointer;
  }
  .tool:hover:not(:disabled) { background: var(--muted); color: var(--foreground); }
  .tool:disabled { opacity: .35; cursor: default; }
  .list { box-sizing: border-box; overflow-y: auto; overscroll-behavior: contain; padding: 4px 10px 8px; scrollbar-width: thin; }
  .inner { position: relative; display: flex; flex-direction: column; gap: 14px; }
  .turn { min-width: 0; }
  @keyframes enter { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { .mini { animation: none; } }
</style>
