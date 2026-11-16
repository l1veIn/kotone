<!-- Adapted from Codeg src/components/ai-elements/reasoning.tsx at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/ThinkingBlock.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: runes, Chinese labels, plain-text reasoning. Open while streaming, folds when the thought ends. -->
<script lang="ts">
  import { ChevronRight, LoaderCircle } from "@lucide/svelte";

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

<details bind:open class="group/thinking text-muted-foreground" data-testid="agent-thinking" data-streaming={streaming}>
  <summary class="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-sm py-0.5 text-xs hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
    {#if streaming}<LoaderCircle class="size-3 animate-spin" />{/if}
    <span>{streaming ? "正在思考…" : "思考过程"}</span>
    <ChevronRight aria-hidden="true" class="size-3 transition-transform group-open/thinking:rotate-90" />
  </summary>
  {#if open}
    <p class="mt-2 mb-0 border-l-2 border-muted pl-3 text-xs leading-relaxed whitespace-pre-wrap">{text}</p>
  {/if}
</details>
