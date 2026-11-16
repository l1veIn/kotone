<!-- Footer adapted from Codeg src/components/message/turn-stats.tsx at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/TurnFooter.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: icon action + stat text after DeepSeek Harness MessageIconActions / StatsPills (MIT); shows the -->
<!-- token usage the provider reported for the turn (hidden when none; never estimated). -->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { Check, Copy } from "@lucide/svelte";
  import type { TokenUsage } from "../agent";
  import { formatTokens } from "./timeline";

  let { copyText = "", endedAtMs = null, usage = null }: { copyText?: string; endedAtMs?: number | null; usage?: TokenUsage | null } = $props();
  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const time = $derived(endedAtMs ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(endedAtMs) : "");
  const usageText = $derived.by(() => {
    if (!usage) return "";
    const parts: string[] = [];
    if (usage.inputTokens !== undefined) parts.push(`输入 ${formatTokens(usage.inputTokens)}${usage.cachedInputTokens ? `（缓存 ${formatTokens(usage.cachedInputTokens)}）` : ""}`);
    if (usage.outputTokens !== undefined) parts.push(`输出 ${formatTokens(usage.outputTokens)}`);
    if (!parts.length) return "";
    const cost = usage.costUsd !== undefined ? ` · 约 $${usage.costUsd.toFixed(4)}` : "";
    return `${parts.join(" · ")} tokens${cost}`;
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(copyText);
      failed = false;
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => { copied = false; }, 2000);
    } catch { failed = true; }
  }
  onDestroy(() => clearTimeout(timer));
</script>

{#if copyText.trim() || time || usageText}
  <footer class="-ml-1 flex flex-wrap items-center gap-1 text-xs text-muted-foreground/80" data-testid="agent-turn-footer">
    {#if copyText.trim()}
      <button type="button" class="flex size-7 items-center justify-center rounded-lg transition-colors hover:bg-muted hover:text-foreground" aria-label={copied ? "已复制" : "复制回答"} title={copied ? "已复制" : "复制回答"} onclick={copy}>
        {#if copied}<Check class="size-3.5 text-success" />{:else}<Copy class="size-3.5" />{/if}
      </button>
    {/if}
    {#if time}<time class="px-1 tabular-nums">{time}</time>{/if}
    {#if usageText}<span class="rounded-full bg-muted/60 px-2 py-0.5 tabular-nums" data-testid="agent-turn-usage">{usageText}</span>{/if}
    {#if failed}<span role="alert">复制失败</span>{/if}
  </footer>
{/if}
