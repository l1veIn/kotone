<script lang="ts">
  // 聊天输入框上方的用量条与详情（照 DeepSeek Harness 的 StatsPills / Statistics：一眼看到命中率与花费，
  // 点开看明细）。今天和本段对话的用量、命中缓存的比例、费用；详情里按天（最近 60 天，点一天看按用途 /
  // 按模型）和本段对话的每次调用。数据来自本机账本 usage.jsonl（usage_summary）与对话里存的用量。
  import { onMount } from "svelte";
  import { BarChart3 } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { usageSummary, type AgentMessage, type ConversationCosts, type TokenUsage, type UsageSummary, type UsageTotals } from "../agent";
  import { conversationUsage, formatCost, formatHitRate, formatTokens, totalsMoney } from "./timeline";

  let { messages = [], running = false, costs = null, compact = false }: {
    messages?: readonly AgentMessage[]; running?: boolean;
    /** 这段对话模型以外的费用（语音、读屏、搜索），来自本机账本。 */
    costs?: ConversationCosts | null;
    /** 放在输入框里面（左下角的一行小字；rc2 玩家反馈：计费挪进输入框）。 */
    compact?: boolean;
  } = $props();

  let summary = $state<UsageSummary | null>(null);
  let open = $state(false);
  let tab = $state<"days" | "conversation">("days");
  let selectedDay = $state<string | null>(null);

  async function load() {
    try { summary = await usageSummary(); } catch { /* 读不到就不显示 */ }
  }
  onMount(() => { void load(); });
  // 每答完一轮刷新一次（正在回答 → 空闲）。
  let wasRunning = false;
  $effect(() => {
    if (wasRunning && !running) void load();
    wasRunning = running;
  });

  const conv = $derived(conversationUsage(messages));
  /** 模型以外的花费（语音、读屏、搜索）：分开列出。 */
  const otherCosts = $derived((costs?.total ?? []).filter((c) => c.category !== "chat"));
  /** 本段对话一共：模型（消息里的用量）+ 模型以外（账本），按币种相加。 */
  const convMoney = $derived.by(() => {
    if (!otherCosts.length) return usageMoney(conv);
    const sum: Record<string, number> = {};
    const add = (currency: string, cost: number) => { sum[currency] = (sum[currency] ?? 0) + cost; };
    if (conv?.cost !== undefined) add(conv.currency ?? "USD", conv.cost);
    else if (conv?.costUsd !== undefined) add("USD", conv.costUsd);
    for (const c of otherCosts) for (const [currency, cost] of Object.entries(c.totals.cost)) add(currency, cost);
    const parts = Object.entries(sum).map(([currency, cost]) => formatCost(cost, currency));
    return parts.length ? parts.join(" + ") : "—";
  });
  /** 分开列：对话 ¥… · 语音 ¥… · 读屏 ¥… · 搜索 ¥…。 */
  const breakdown = $derived(otherCosts.length
    ? [...(conv && usageMoney(conv) !== "—" ? [`对话 ${usageMoney(conv)}`] : []), ...otherCosts.map((c) => `${c.label} ${totalsMoney(c.totals) || "—"}`)].join(" · ")
    : "");
  function money(t: UsageTotals | undefined): string {
    if (!t) return "—";
    const parts = Object.entries(t.cost).map(([currency, cost]) => formatCost(cost, currency));
    return parts.length ? parts.join(" + ") : "—";
  }
  function costValue(t: UsageTotals): number {
    return Object.values(t.cost).reduce((a, b) => a + b, 0);
  }
  function usageMoney(u: TokenUsage | null): string {
    if (!u) return "—";
    return u.cost !== undefined ? formatCost(u.cost, u.currency) : u.costUsd !== undefined ? formatCost(u.costUsd, "USD") : "—";
  }
  const days = $derived([...(summary?.days ?? [])].reverse());
  const maxCost = $derived(Math.max(0.000001, ...days.map((d) => costValue(d.totals))));
  const dayDetail = $derived(days.find((d) => d.day === selectedDay) ?? null);
  /** 本段对话里每一次花了钱的调用（模型回复与整理摘要），新的在前。 */
  const calls = $derived(messages
    .filter((m): m is Extract<AgentMessage, { role: "assistant" | "notice" }> => (m.role === "assistant" || m.role === "notice") && !!m.usage)
    .map((m) => ({ at: m.atMs, kind: m.role === "notice" ? (m.recap ? "复盘" : "整理摘要") : "回答", usage: m.usage! }))
    .reverse());
  const time = (ms: number) => new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(ms);
  function extras(t: UsageTotals): string {
    const parts: string[] = [];
    if (t.characters) parts.push(`合成 ${formatTokens(t.characters)} 字`);
    if (t.audioMs) parts.push(`识别 ${(t.audioMs / 60000).toFixed(1)} 分钟`);
    if (t.images) parts.push(`读屏 ${t.images} 张`);
    return parts.join(" · ");
  }
</script>

{#if compact && (summary || conv || otherCosts.length)}
  <button type="button" class="flex h-8 max-w-full min-w-0 items-center gap-1.5 overflow-hidden rounded-full px-2 text-left text-[11px] whitespace-nowrap tabular-nums text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
    onclick={() => { open = true; void load(); }} data-testid="usage-pill" aria-label="查看用量与费用" title="用量与费用：点开看明细">
    <BarChart3 class="size-3 shrink-0" />
    {#if summary}
      <span class="shrink-0" data-testid="usage-pill-today">{`今天 ${money(summary.today)}${summary.today.inputTokens ? ` · 命中 ${formatHitRate(summary.today.cacheHitTokens, summary.today.inputTokens)}` : ""}`}</span>
    {/if}
    {#if conv?.inputTokens}
      <span class="min-w-0 truncate" data-testid="usage-pill-conversation">本段对话 {convMoney} · 命中 {formatHitRate(conv.cachedInputTokens, conv.inputTokens)}</span>
    {:else if otherCosts.length}
      <span class="min-w-0 truncate" data-testid="usage-pill-conversation">本段对话 {convMoney}</span>
    {/if}
  </button>
{:else if summary || conv || otherCosts.length}
  <button type="button" class="flex w-full flex-wrap items-center gap-x-3 gap-y-0.5 rounded-lg px-2 py-1 text-left text-xs tabular-nums text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
    onclick={() => { open = true; void load(); }} data-testid="usage-pill" aria-label="查看用量与费用">
    <BarChart3 class="size-3.5 shrink-0" />
    {#if summary}
      <span data-testid="usage-pill-today">{`今天 ${money(summary.today)}${summary.today.inputTokens ? ` · 命中 ${formatHitRate(summary.today.cacheHitTokens, summary.today.inputTokens)}` : ""}`}</span>
    {/if}
    {#if conv?.inputTokens}
      <span data-testid="usage-pill-conversation">本段对话 {convMoney} · 命中 {formatHitRate(conv.cachedInputTokens, conv.inputTokens)}</span>
    {:else if otherCosts.length}
      <span data-testid="usage-pill-conversation">本段对话 {convMoney}</span>
    {/if}
    <span class="ml-auto text-primary/80">明细 →</span>
  </button>
{/if}

<Dialog.Root bind:open>
  <Dialog.Content class="ui-scope max-h-[85vh] w-[calc(100vw-2rem)] overflow-hidden sm:max-w-[780px]" data-testid="usage-dialog">
    <Dialog.Header>
      <Dialog.Title>用量与费用</Dialog.Title>
      <Dialog.Description>命中缓存的输入便宜得多（DeepSeek 约为没命中的 1/50）。费用是按价格表估的，以服务商账单为准。</Dialog.Description>
    </Dialog.Header>
    <div class="flex gap-1 rounded-lg bg-muted/60 p-1 text-[13px]" role="tablist">
      {#each [{ id: "days", label: "按天" }, { id: "conversation", label: "本段对话" }] as t (t.id)}
        <button type="button" role="tab" aria-selected={tab === t.id} data-testid={`usage-tab-${t.id}`}
          class="flex-1 rounded-md px-3 py-1 {tab === t.id ? 'bg-background font-medium shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => { tab = t.id as typeof tab; }}>{t.label}</button>
      {/each}
    </div>
    <div class="min-h-0 overflow-y-auto pr-1" style="max-height: 60vh">
      {#if tab === "days"}
        {#if summary}
          <div class="mb-3 grid grid-cols-2 gap-2 text-xs tabular-nums">
            {#each [{ label: "今天", t: summary.today }, { label: "本月", t: summary.month }] as card (card.label)}
              <div class="rounded-lg border border-border/80 p-2.5">
                <div class="flex items-baseline justify-between"><span class="text-muted-foreground">{card.label}</span><span class="text-base font-semibold">{money(card.t)}</span></div>
                <div class="mt-0.5 text-muted-foreground">命中 {formatHitRate(card.t.cacheHitTokens, card.t.inputTokens) || "—"} · {card.t.requests} 次 · 输入 {formatTokens(card.t.inputTokens)} · 输出 {formatTokens(card.t.outputTokens)}</div>
              </div>
            {/each}
          </div>
        {/if}
        {#if days.length}
          <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid="usage-days">
            <thead class="text-left text-muted-foreground">
              <tr><th class="py-1 font-normal">日期</th><th class="font-normal">调用</th><th class="font-normal">输入</th><th class="font-normal">命中</th><th class="font-normal">输出</th><th class="font-normal">语音 / 读屏</th><th class="w-32 text-right font-normal">费用</th></tr>
            </thead>
            <tbody>
              {#each days as d (d.day)}
                <tr class="cursor-pointer border-t border-border/60 hover:bg-muted/40 {selectedDay === d.day ? 'bg-muted/50' : ''}" data-testid="usage-day" data-day={d.day}
                  onclick={() => { selectedDay = selectedDay === d.day ? null : d.day; }}>
                  <td class="py-1.5">{d.day.slice(5)}</td>
                  <td>{d.totals.requests}</td>
                  <td>{formatTokens(d.totals.inputTokens)}</td>
                  <td>{formatHitRate(d.totals.cacheHitTokens, d.totals.inputTokens) || "—"}</td>
                  <td>{formatTokens(d.totals.outputTokens)}</td>
                  <td class="max-w-44 truncate text-muted-foreground" title={extras(d.totals)}>{extras(d.totals) || "—"}</td>
                  <td class="text-right">
                    <div class="flex items-center justify-end gap-2">
                      <span class="h-1.5 rounded-full bg-primary/60" style:width={`${Math.max(2, (costValue(d.totals) / maxCost) * 56)}px`}></span>
                      <span>{money(d.totals)}</span>
                    </div>
                  </td>
                </tr>
                {#if selectedDay === d.day && dayDetail}
                  <tr data-testid="usage-day-detail"><td colspan="7" class="pb-2 whitespace-normal">
                    <div class="mt-1 grid gap-2 rounded-lg bg-muted/40 p-2.5 sm:grid-cols-2">
                      <div><p class="m-0 mb-1 font-medium text-foreground/80">按用途</p>
                        {#each dayDetail.byKind as k (k.kind)}<p class="m-0 flex justify-between gap-2"><span>{`${k.label} · ${k.totals.requests} 次${k.totals.inputTokens ? ` · 命中 ${formatHitRate(k.totals.cacheHitTokens, k.totals.inputTokens)}` : ""}`}</span><span>{money(k.totals)}</span></p>{/each}
                      </div>
                      <div><p class="m-0 mb-1 font-medium text-foreground/80">按模型</p>
                        {#each dayDetail.byModel as m (m.model)}<p class="m-0 flex justify-between gap-2"><span class="truncate">{m.model || "—"} · {m.totals.requests} 次</span><span>{money(m.totals)}</span></p>{/each}
                      </div>
                    </div>
                  </td></tr>
                {/if}
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="m-0 py-6 text-center text-xs text-muted-foreground">最近还没有用量。</p>
        {/if}
        {#if summary}<p class="m-0 mt-2 text-[11px] text-muted-foreground/80">{summary.note} 价格：{summary.priceSource}。</p>{/if}
      {:else}
        {#if conv}
          <p class="m-0 mb-2 text-xs tabular-nums text-muted-foreground" data-testid="usage-conversation-total">本段对话：输入 {formatTokens(conv.inputTokens ?? 0)}（命中 {formatHitRate(conv.cachedInputTokens, conv.inputTokens) || "—"}）· 输出 {formatTokens(conv.outputTokens ?? 0)} · {convMoney}</p>
        {/if}
        {#if breakdown}
          <p class="m-0 mb-2 text-xs tabular-nums text-muted-foreground" data-testid="usage-conversation-breakdown">{breakdown}</p>
        {/if}
        {#if calls.length}
          <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid="usage-calls">
            <thead class="text-left text-muted-foreground"><tr><th class="py-1 font-normal">时间</th><th class="font-normal">什么</th><th class="font-normal">输入</th><th class="font-normal">命中</th><th class="font-normal">输出</th><th class="text-right font-normal">费用</th></tr></thead>
            <tbody>
              {#each calls as c, i (i)}
                {@const rate = c.usage.inputTokens ? (c.usage.cachedInputTokens ?? 0) / c.usage.inputTokens : null}
                <tr class="border-t border-border/60" data-testid="usage-call">
                  <td class="py-1.5">{time(c.at)}</td><td>{c.kind}</td><td>{formatTokens(c.usage.inputTokens ?? 0)}</td>
                  <td class={rate !== null && rate < 0.5 ? "text-warning" : ""} title={rate !== null && rate < 0.5 ? "这次基本没命中缓存：通常是一段对话的第一次请求，或刚整理过摘要" : ""}>{formatHitRate(c.usage.cachedInputTokens, c.usage.inputTokens) || "—"}</td>
                  <td>{formatTokens(c.usage.outputTokens ?? 0)}</td><td class="text-right">{usageMoney(c.usage)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="m-0 py-6 text-center text-xs text-muted-foreground">这段对话还没有用量记录（服务商没报用量时不显示）。</p>
        {/if}
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
