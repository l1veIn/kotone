<!-- Adapted from Codeg src/components/ai-elements/reasoning.tsx at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/ThinkingBlock.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: runes, Chinese labels, plain-text reasoning, one-line flow row after DeepSeek Harness -->
<!-- ReasoningRow (MIT). Open while streaming, folds when the thought ends. -->
<script lang="ts">
  import { Brain, ChevronRight } from "@lucide/svelte";

  let { text, streaming = false }: { text: string; streaming?: boolean } = $props();
  let open = $state(false);
  let previous: boolean | null = null;
  $effect.pre(() => {
    if (streaming !== previous) {
      open = streaming;
      previous = streaming;
    }
  });
</script>

<details bind:open class="group/thinking min-w-0" data-testid="agent-thinking" data-streaming={streaming}>
  <summary class="flex h-7 w-fit cursor-pointer list-none items-center gap-2 rounded-md pr-1.5 text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
    <Brain class="size-3.5 shrink-0" />
    <span class={streaming ? "kotone-shimmer" : ""}>{streaming ? "正在思考…" : "思考过程"}</span>
    <ChevronRight aria-hidden="true" class="size-3.5 opacity-0 transition-[opacity,transform] group-hover/thinking:opacity-100 group-open/thinking:rotate-90 group-open/thinking:opacity-100" />
  </summary>
  {#if open}
    <p class="mt-1 mb-2 ml-[7px] border-l border-hairline pl-4 text-[13px] leading-6 whitespace-pre-wrap text-muted-foreground">{text}</p>
  {/if}
</details>
