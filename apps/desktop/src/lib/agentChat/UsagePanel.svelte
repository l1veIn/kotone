<script lang="ts">
  // 输入框只显示今日花费。详情里的历史用量来自本机账本和对话记录，余额由当前 API 连接的服务商提供。
  import { onMount } from "svelte";
  import { BarChart3 } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { m } from "$lib/paraglide/messages.js";
  import { getLocale } from "$lib/paraglide/runtime.js";
  import { usageBalance, usageSummary, type AgentMessage, type ConversationCosts, type TokenUsage, type UsageBalance, type UsageSummary, type UsageTotals } from "../agent";
  import { conversationUsage, formatCost, formatHitRate, formatTokens, totalsMoney } from "./timeline";

  let { messages = [], running = false, costs = null }: {
    messages?: readonly AgentMessage[]; running?: boolean;
    /** 这段对话模型以外的费用（语音、读屏、搜索），来自本机账本。 */
    costs?: ConversationCosts | null;
  } = $props();

  let summary = $state<UsageSummary | null>(null);
  let balanceRequest = $state<Promise<UsageBalance | null | undefined> | null>(null);
  let open = $state(false);
  let tab = $state<"days" | "conversation">("days");
  let selectedDay = $state<string | null>(null);

  async function load() {
    try { summary = await usageSummary(); } catch { /* 读不到就不显示 */ }
  }
  function showDetails() {
    open = true;
    void load();
    balanceRequest = usageBalance().catch(() => undefined);
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
    ? [...(conv && usageMoney(conv) !== "—" ? [m.chat_usage_chat_cost({ money: usageMoney(conv) })] : []), ...otherCosts.map((c) => `${c.label} ${totalsMoney(c.totals) || "—"}`)].join(" · ")
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
    .map((msg) => ({ at: msg.atMs, kind: msg.role === "notice" ? (msg.recap ? m.chat_usage_call_recap() : m.chat_usage_call_compaction()) : m.chat_usage_call_answer(), usage: msg.usage! }))
    .reverse());
  const time = (ms: number) => new Intl.DateTimeFormat(getLocale(), { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(ms);
  function extras(t: UsageTotals): string {
    const parts: string[] = [];
    if (t.characters) parts.push(m.chat_usage_extra_tts({ count: formatTokens(t.characters) }));
    if (t.audioMs) parts.push(m.chat_usage_extra_asr({ minutes: (t.audioMs / 60000).toFixed(1) }));
    if (t.images) parts.push(m.chat_usage_extra_screen({ count: t.images }));
    return parts.join(" · ");
  }
</script>

<button type="button" class="flex h-8 max-w-full min-w-0 items-center gap-1.5 overflow-hidden rounded-full px-2 text-left text-[11px] tabular-nums text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
  onclick={showDetails} data-testid="usage-pill" aria-label={m.chat_usage_pill_label()} title={m.chat_usage_pill_title()}>
  <BarChart3 class="size-3 shrink-0" aria-hidden="true" />
  <span class="min-w-0 truncate" data-testid="usage-pill-today">{m.chat_usage_today({ money: money(summary?.today) })}</span>
</button>

<Dialog.Root bind:open>
  <Dialog.Content class="ui-scope max-h-[85vh] w-[calc(100vw-2rem)] overflow-hidden sm:max-w-[780px]" data-testid="usage-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.chat_usage_dialog_title()}</Dialog.Title>
      <Dialog.Description>{m.chat_usage_dialog_description()}</Dialog.Description>
    </Dialog.Header>
    <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1 rounded-lg border border-border/80 px-3 py-2 text-xs tabular-nums"
      aria-live="polite" data-testid="usage-balance">
      <span class="font-medium">{m.chat_usage_balance()}</span>
      {#await balanceRequest}
        <span class="ml-auto text-muted-foreground">{m.chat_usage_balance_loading()}</span>
      {:then balance}
        {#if balance}
          <span class="min-w-0 flex-1 truncate text-muted-foreground" title={balance.connectionName}>{balance.connectionName}</span>
          <span class="ml-auto flex flex-wrap gap-x-3 font-semibold" data-testid="usage-balance-amounts">
            {#each balance.balances as { currency, amount } (currency)}
              <span>{currency === "CNY" ? "¥" : "$"}{amount}</span>
            {/each}
          </span>
        {:else}
          <span class="ml-auto text-muted-foreground">{balance === undefined ? m.chat_usage_balance_failed() : m.chat_usage_balance_unavailable()}</span>
        {/if}
      {/await}
    </div>
    <div class="flex gap-1 rounded-lg bg-muted/60 p-1 text-[13px]" role="tablist">
      {#each [{ id: "days", label: m.chat_usage_tab_days() }, { id: "conversation", label: m.chat_usage_tab_conversation() }] as t (t.id)}
        <button type="button" role="tab" aria-selected={tab === t.id} data-testid={`usage-tab-${t.id}`}
          class="flex-1 rounded-md px-3 py-1 {tab === t.id ? 'bg-background font-medium shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => { tab = t.id as typeof tab; }}>{t.label}</button>
      {/each}
    </div>
    <div class="min-h-0 overflow-y-auto pr-1" style="max-height: 60vh">
      {#if tab === "days"}
        {#if summary}
          <div class="mb-3 grid grid-cols-2 gap-2 text-xs tabular-nums">
            {#each [{ label: m.chat_usage_card_today(), t: summary.today }, { label: m.chat_usage_card_month(), t: summary.month }] as card (card.label)}
              <div class="rounded-lg border border-border/80 p-2.5">
                <div class="flex items-baseline justify-between"><span class="text-muted-foreground">{card.label}</span><span class="text-base font-semibold">{money(card.t)}</span></div>
                <div class="mt-0.5 text-muted-foreground">{m.chat_usage_card_stats({ rate: formatHitRate(card.t.cacheHitTokens, card.t.inputTokens) || "—", requests: card.t.requests, input: formatTokens(card.t.inputTokens), output: formatTokens(card.t.outputTokens) })}</div>
              </div>
            {/each}
          </div>
        {/if}
        {#if days.length}
          <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid="usage-days">
            <thead class="text-left text-muted-foreground">
              <tr><th class="py-1 font-normal">{m.chat_usage_col_date()}</th><th class="font-normal">{m.chat_usage_col_requests()}</th><th class="font-normal">{m.chat_usage_col_input()}</th><th class="font-normal">{m.chat_usage_col_hit()}</th><th class="font-normal">{m.chat_usage_col_output()}</th><th class="font-normal">{m.chat_usage_col_extras()}</th><th class="w-32 text-right font-normal">{m.chat_usage_col_cost()}</th></tr>
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
                      <div><p class="m-0 mb-1 font-medium text-foreground/80">{m.chat_usage_by_kind()}</p>
                        {#each dayDetail.byKind as k (k.kind)}<p class="m-0 flex justify-between gap-2"><span>{`${k.label} · ${m.chat_usage_requests({ count: k.totals.requests })}${k.totals.inputTokens ? ` · ${m.chat_usage_hit({ rate: formatHitRate(k.totals.cacheHitTokens, k.totals.inputTokens) })}` : ""}`}</span><span>{money(k.totals)}</span></p>{/each}
                      </div>
                      <div><p class="m-0 mb-1 font-medium text-foreground/80">{m.chat_usage_by_model()}</p>
                        {#each dayDetail.byModel as model (model.model)}<p class="m-0 flex justify-between gap-2"><span class="truncate">{model.model || "—"} · {m.chat_usage_requests({ count: model.totals.requests })}</span><span>{money(model.totals)}</span></p>{/each}
                      </div>
                    </div>
                  </td></tr>
                {/if}
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="m-0 py-6 text-center text-xs text-muted-foreground">{m.chat_usage_days_empty()}</p>
        {/if}
        {#if summary}<p class="m-0 mt-2 text-[11px] text-muted-foreground/80">{m.chat_usage_price_note({ note: summary.note, source: summary.priceSource })}</p>{/if}
      {:else}
        {#if conv}
          <p class="m-0 mb-2 text-xs tabular-nums text-muted-foreground" data-testid="usage-conversation-total">{m.chat_usage_conversation_total({ input: formatTokens(conv.inputTokens ?? 0), rate: formatHitRate(conv.cachedInputTokens, conv.inputTokens) || "—", output: formatTokens(conv.outputTokens ?? 0), money: convMoney })}</p>
        {:else if otherCosts.length}
          <p class="m-0 mb-2 text-xs tabular-nums text-muted-foreground" data-testid="usage-conversation-total">{m.chat_usage_conversation({ money: convMoney })}</p>
        {/if}
        {#if breakdown}
          <p class="m-0 mb-2 text-xs tabular-nums text-muted-foreground" data-testid="usage-conversation-breakdown">{breakdown}</p>
        {/if}
        {#if calls.length}
          <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid="usage-calls">
            <thead class="text-left text-muted-foreground"><tr><th class="py-1 font-normal">{m.chat_usage_col_time()}</th><th class="font-normal">{m.chat_usage_col_kind()}</th><th class="font-normal">{m.chat_usage_col_input()}</th><th class="font-normal">{m.chat_usage_col_hit()}</th><th class="font-normal">{m.chat_usage_col_output()}</th><th class="text-right font-normal">{m.chat_usage_col_cost()}</th></tr></thead>
            <tbody>
              {#each calls as c, i (i)}
                {@const rate = c.usage.inputTokens ? (c.usage.cachedInputTokens ?? 0) / c.usage.inputTokens : null}
                <tr class="border-t border-border/60" data-testid="usage-call">
                  <td class="py-1.5">{time(c.at)}</td><td>{c.kind}</td><td>{formatTokens(c.usage.inputTokens ?? 0)}</td>
                  <td class={rate !== null && rate < 0.5 ? "text-warning" : ""} title={rate !== null && rate < 0.5 ? m.chat_usage_low_hit_title() : ""}>{formatHitRate(c.usage.cachedInputTokens, c.usage.inputTokens) || "—"}</td>
                  <td>{formatTokens(c.usage.outputTokens ?? 0)}</td><td class="text-right">{usageMoney(c.usage)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="m-0 py-6 text-center text-xs text-muted-foreground">{m.chat_usage_calls_empty()}</p>
        {/if}
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
