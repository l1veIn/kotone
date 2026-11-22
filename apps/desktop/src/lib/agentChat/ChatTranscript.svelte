<!-- Reverse-scroll and follow-latest behaviour adapted from Codeg virtualized-message-thread at -->
<!-- 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, via RambleDesk apps/desktop/src/lib/agents/chat/SessionTranscript.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: runes; the whole conversation is local, so the window only reveals more turns (20 at a time). -->
<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import type { AgentAsk, Answer } from "../agent";
  import ChatTurn from "./ChatTurn.svelte";
  import { captureScrollAnchor, crossedHistoryThreshold, restoreScrollAnchor } from "./scrollAnchor";
  import type { Turn } from "./timeline";

  const PAGE = 20;
  let { conversationKey, turns, askFor, onAnswer, empty }: {
    /** 换了对话就复位窗口与滚动。 */
    conversationKey: string;
    turns: Turn[];
    askFor: (callId: string) => AgentAsk | undefined;
    onAnswer: (askId: string, answer: Answer) => Promise<void>;
    empty?: Snippet;
  } = $props();

  let viewport = $state<HTMLDivElement>();
  let limit = $state(PAGE);
  let stickToBottom = true;
  let previousTop: number | null = null;
  let activeKey = "";

  const hidden = $derived(Math.max(0, turns.length - limit));
  const visible = $derived(turns.slice(hidden));

  $effect.pre(() => {
    if (conversationKey !== activeKey) {
      activeKey = conversationKey;
      limit = PAGE;
      stickToBottom = true;
      previousTop = null;
    }
  });

  // 新内容到来时：如果用户停在底部附近，就跟到最新。
  $effect(() => {
    void turns;
    void followLatest();
  });

  async function followLatest() {
    await tick();
    if (stickToBottom && viewport) {
      viewport.scrollTop = viewport.scrollHeight;
      previousTop = viewport.scrollTop;
    }
  }

  function onScroll() {
    if (!viewport) return;
    const top = viewport.scrollTop;
    const load = crossedHistoryThreshold(previousTop, top);
    previousTop = top;
    stickToBottom = viewport.scrollHeight - top - viewport.clientHeight < 80;
    if (load && hidden) void revealOlder();
  }

  async function revealOlder() {
    if (!viewport || !hidden) return;
    const element = viewport;
    const anchor = captureScrollAnchor(element);
    stickToBottom = false;
    limit += PAGE;
    await tick();
    if (anchor) restoreScrollAnchor(element, anchor);
    previousTop = element.scrollTop;
  }

  function observeSize(element: HTMLElement) {
    const observer = new ResizeObserver(() => void followLatest());
    observer.observe(element);
    return { destroy() { observer.disconnect(); } };
  }
</script>

<div bind:this={viewport} onscroll={onScroll} class="min-h-0 flex-1 overflow-y-auto overscroll-contain py-6 [scrollbar-gutter:stable]" data-testid="agent-transcript" aria-label={m.chat_transcript_label()}>
  {#if turns.length === 0}
    {@render empty?.()}
  {:else}
    <div class="mx-auto w-full max-w-[808px] space-y-8 px-6" use:observeSize>
      {#if hidden}
        <div class="flex justify-center">
          <Button variant="ghost" size="sm" data-testid="agent-load-earlier" onclick={() => void revealOlder()}>{m.chat_transcript_load_earlier({ count: hidden })}</Button>
        </div>
      {/if}
      {#each visible as turn (turn.id)}
        <ChatTurn {turn} {askFor} {onAnswer} />
      {/each}
    </div>
  {/if}
</div>
