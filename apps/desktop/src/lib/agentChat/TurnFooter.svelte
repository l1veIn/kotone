<!-- Footer adapted from Codeg src/components/message/turn-stats.tsx at 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, -->
<!-- via RambleDesk apps/desktop/src/lib/agents/chat/TurnFooter.svelte. -->
<!-- Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0 -->
<!-- Kotone: icon action + stat text after DeepSeek Harness MessageIconActions / StatsPills (MIT); shows the -->
<!-- token usage the provider reported for the turn (hidden when none; never estimated). Costs outside the model -->
<!-- (speech, screen reading, web search; from the local ledger) are listed separately: “对话 ¥… · 语音 ¥… · 读屏 ¥…”. -->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { Check, Copy } from "@lucide/svelte";
  import type { CategoryTotals, TokenUsage } from "../agent";
  import { formatCost, formatHitRate, formatTokens, totalsMoney } from "./timeline";

  let { copyText = "", endedAtMs = null, usage = null, extraCosts = [] }: {
    copyText?: string; endedAtMs?: number | null; usage?: TokenUsage | null;
    /** 模型以外的费用（语音、读屏、搜索），按类别。 */
    extraCosts?: CategoryTotals[];
  } = $props();
  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const time = $derived(endedAtMs ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(endedAtMs) : "");
  const usageText = $derived.by(() => {
    const parts: string[] = [];
    if (usage?.inputTokens !== undefined) {
      const rate = usage.cachedInputTokens !== undefined ? formatHitRate(usage.cachedInputTokens, usage.inputTokens) : "";
      parts.push(`输入 ${formatTokens(usage.inputTokens)}${rate ? `（命中缓存 ${rate}）` : ""}`);
    }
    if (usage?.outputTokens !== undefined) parts.push(`输出 ${formatTokens(usage.outputTokens)}`);
    const tokens = parts.length ? `${parts.join(" · ")} tokens` : "";
    const money = !usage ? "" : usage.cost !== undefined ? formatCost(usage.cost, usage.currency) : formatCost(usage.costUsd, "USD");
    const extras = extraCosts.map((c) => `${c.label} ${totalsMoney(c.totals) || "—"}`);
    if (!extras.length) return tokens ? `${tokens}${money ? ` · 约 ${money}` : ""}` : "";
    // 模型以外还有花费：分开列出（对话 ¥… · 语音 ¥… · 读屏 ¥…）。
    const costs = [...(money ? [`对话 ${money}`] : []), ...extras].join(" · ");
    return tokens ? `${tokens} · ${costs}` : costs;
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
