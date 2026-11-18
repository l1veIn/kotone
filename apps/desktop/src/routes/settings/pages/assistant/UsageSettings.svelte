<script lang="ts">
  // 「琴音 → 设置 → 用量」：今天 / 本月花了多少 token 和钱、前缀缓存命中多少（usage.jsonl）。
  // 命中率的算法与显示照 DeepSeek Harness（MIT）的 StatsPills / token-format：命中 ÷ 输入，没全中不显示 100%。
  // 费用是 Kotone 按价格表估的（DeepSeek 分高峰 / 空闲），以服务商账单为准。
  import { onMount } from "svelte";
  import { RefreshCw } from "@lucide/svelte";
  import { usageSummary, type UsageSummary, type UsageTotals } from "../../../../lib/agent";
  import { formatCost, formatHitRate, formatTokens } from "../../../../lib/agentChat/timeline";
  import { errText, patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";

  let summary = $state<UsageSummary | null>(null);
  let error = $state("");
  let loading = $state(false);
  let savingCompact = $state(false);
  const compactAt = $derived($settingsStore?.agent?.compactAtTokens ?? 64000);

  async function load() {
    loading = true;
    try { summary = await usageSummary(); error = ""; }
    catch (e) { error = `读取用量失败：${errText(e)}`; }
    finally { loading = false; }
  }
  onMount(() => { void load(); });

  function money(t: UsageTotals): string {
    const parts = Object.entries(t.cost).map(([currency, cost]) => formatCost(cost, currency));
    return parts.length ? parts.join(" + ") : "—";
  }
  function minutes(ms: number): string {
    return ms >= 60_000 ? `${(ms / 60_000).toFixed(1)} 分钟` : `${Math.round(ms / 1000)} 秒`;
  }
  function details(t: UsageTotals): string {
    const parts: string[] = [];
    if (t.inputTokens || t.outputTokens) parts.push(`输入 ${formatTokens(t.inputTokens)} · 输出 ${formatTokens(t.outputTokens)} tokens`);
    if (t.characters) parts.push(`语音合成 ${formatTokens(t.characters)} 字`);
    if (t.audioMs) parts.push(`语音识别 ${minutes(t.audioMs)}`);
    if (t.images) parts.push(`文字识别 ${t.images} 张`);
    return parts.join(" · ") || "还没有用量";
  }
  async function saveCompact(value: number) {
    if (savingCompact || !Number.isFinite(value) || value < 0) return;
    savingCompact = true;
    try { await patchSettings({ agent: { compactAtTokens: Math.round(value) } }, "已保存"); }
    finally { savingCompact = false; }
  }
</script>

<div class="space-y-6" data-testid="usage-settings">
  <SettingsSection title="用量与费用" id="companion-usage-title"
    description="对话、整理摘要、在线语音和文字识别花了多少。命中缓存的输入便宜得多（DeepSeek 约为没命中的 1/50）。">
    <div class="flex items-center justify-between gap-3 py-2">
      <p class="m-0 text-xs text-muted-foreground">{summary?.note ?? ""}</p>
      <Button size="sm" variant="ghost" disabled={loading} onclick={() => void load()} aria-label="刷新用量"><RefreshCw class="size-3.5" />刷新</Button>
    </div>
    {#if error}<p class="m-0 text-xs text-destructive" role="alert">{error}</p>{/if}
    {#if summary}
      <div class="grid gap-3 py-2 sm:grid-cols-2">
        {#each [{ label: "今天", t: summary.today, id: "today" }, { label: "本月", t: summary.month, id: "month" }] as card (card.id)}
          <div class="rounded-xl border border-border/80 bg-muted/30 p-3" data-testid={`usage-${card.id}`}>
            <div class="flex items-baseline justify-between gap-2">
              <span class="text-xs text-muted-foreground">{card.label}</span>
              <span class="text-lg font-semibold tabular-nums">{money(card.t)}</span>
            </div>
            <p class="m-0 mt-1 text-xs tabular-nums text-muted-foreground">
              命中缓存 <span class="font-medium text-foreground" data-testid={`usage-${card.id}-hit`}>{formatHitRate(card.t.cacheHitTokens, card.t.inputTokens) || "—"}</span>
              · {card.t.requests} 次调用
            </p>
            <p class="m-0 mt-1 text-xs leading-relaxed text-muted-foreground">{details(card.t)}</p>
            {#if card.t.unpriced}<p class="m-0 mt-1 text-[11px] text-muted-foreground/80">其中 {card.t.unpriced} 次没有价格（价格表里没有这个模型），只算了 token。</p>{/if}
          </div>
        {/each}
      </div>
      {#if summary.byKind.length}
        <div class="py-2">
          <p class="m-0 mb-1.5 text-xs font-medium text-foreground/80">本月按用途</p>
          <ul class="m-0 list-none space-y-1 p-0 text-xs tabular-nums text-muted-foreground" data-testid="usage-by-kind">
            {#each summary.byKind as item (item.kind)}
              <li class="flex flex-wrap justify-between gap-2"><span class="text-foreground/85">{item.label}</span><span>{details(item.totals)}{item.totals.inputTokens ? ` · 命中 ${formatHitRate(item.totals.cacheHitTokens, item.totals.inputTokens)}` : ""} · {money(item.totals)}</span></li>
            {/each}
          </ul>
        </div>
      {/if}
      <p class="m-0 py-2 text-[11px] leading-relaxed text-muted-foreground/80">价格：{summary.priceSource}。{#if summary.ledger}账本：{summary.ledger}{/if}</p>
    {/if}
    <SettingsRow label="对话多长时整理成摘要" description="一段对话估到这么多 token 时，把较早的部分整理成摘要，之后只看摘要和最近几轮（整理那次多花一点，之后又能命中缓存）。0 = 不整理。">
      {#snippet control()}
        <input type="number" min="0" step="1000" class="h-8 w-32 rounded-md border border-input bg-background px-2 text-sm tabular-nums" aria-label="整理阈值（token）"
          value={compactAt} disabled={savingCompact} onchange={(e) => void saveCompact(Number(e.currentTarget.value))} />
      {/snippet}
    </SettingsRow>
  </SettingsSection>
</div>
