<script lang="ts">
  // 「琴音 → 设置 → 用量」：今天 / 本月花了多少 token 和钱、前缀缓存命中多少（usage.jsonl）。
  // 命中率的算法与显示照 DeepSeek Harness（MIT）的 StatsPills / token-format：命中 ÷ 输入，没全中不显示 100%。
  // 费用是 Kotone 按价格表估的（DeepSeek 分高峰 / 空闲），以服务商账单为准。
  import { onMount } from "svelte";
  import { RefreshCw } from "@lucide/svelte";
  import { usageSummary, type UsageSummary, type UsageTotals } from "../../../../lib/agent";
  import { formatCost, formatHitRate, formatTokens } from "../../../../lib/agentChat/timeline";
  import { errText, patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
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
    catch (e) { error = m.companion_usage_load_failed({ error: errText(e) }); }
    finally { loading = false; }
  }
  onMount(() => { void load(); });

  function money(t: UsageTotals): string {
    const parts = Object.entries(t.cost).map(([currency, cost]) => formatCost(cost, currency));
    return parts.length ? parts.join(" + ") : "—";
  }
  function minutes(ms: number): string {
    return ms >= 60_000 ? m.companion_usage_minutes({ minutes: (ms / 60_000).toFixed(1) }) : m.companion_usage_seconds({ seconds: Math.round(ms / 1000) });
  }
  function details(t: UsageTotals): string {
    const parts: string[] = [];
    if (t.inputTokens || t.outputTokens) parts.push(m.companion_usage_tokens({ input: formatTokens(t.inputTokens), output: formatTokens(t.outputTokens) }));
    if (t.characters) parts.push(m.companion_usage_tts({ characters: formatTokens(t.characters) }));
    if (t.audioMs) parts.push(m.companion_usage_asr({ duration: minutes(t.audioMs) }));
    if (t.images) parts.push(m.companion_usage_ocr({ images: t.images }));
    return parts.join(" · ") || m.companion_usage_none();
  }
  async function saveCompact(value: number) {
    if (savingCompact || !Number.isFinite(value) || value < 0) return;
    savingCompact = true;
    try { await patchSettings({ agent: { compactAtTokens: Math.round(value) } }, m.companion_saved()); }
    finally { savingCompact = false; }
  }
</script>

<div class="space-y-6" data-testid="usage-settings">
  <SettingsSection title={m.companion_usage_title()} id="companion-usage-title"
    description={m.companion_usage_desc()}>
    <div class="flex items-center justify-between gap-3 py-2">
      <p class="m-0 text-xs text-muted-foreground">{summary?.note ?? ""}</p>
      <Button size="sm" variant="ghost" disabled={loading} onclick={() => void load()} aria-label={m.companion_usage_refresh_aria()}><RefreshCw class="size-3.5" />{m.companion_usage_refresh()}</Button>
    </div>
    {#if error}<p class="m-0 text-xs text-destructive" role="alert">{error}</p>{/if}
    {#if summary}
      <div class="grid gap-3 py-2 sm:grid-cols-2">
        {#each [{ label: m.companion_usage_today(), t: summary.today, id: "today" }, { label: m.companion_usage_month(), t: summary.month, id: "month" }] as card (card.id)}
          <div class="rounded-xl border border-border/80 bg-muted/30 p-3" data-testid={`usage-${card.id}`}>
            <div class="flex items-baseline justify-between gap-2">
              <span class="text-xs text-muted-foreground">{card.label}</span>
              <span class="text-lg font-semibold tabular-nums">{money(card.t)}</span>
            </div>
            <p class="m-0 mt-1 text-xs tabular-nums text-muted-foreground">
              {m.companion_usage_cache_hit()} <span class="font-medium text-foreground" data-testid={`usage-${card.id}-hit`}>{formatHitRate(card.t.cacheHitTokens, card.t.inputTokens) || "—"}</span>
              {m.companion_usage_requests({ count: card.t.requests })}
            </p>
            <p class="m-0 mt-1 text-xs leading-relaxed text-muted-foreground">{details(card.t)}</p>
            {#if card.t.unpriced}<p class="m-0 mt-1 text-[11px] text-muted-foreground/80">{m.companion_usage_unpriced({ count: card.t.unpriced })}</p>{/if}
          </div>
        {/each}
      </div>
      {#if summary.byKind.length}
        <div class="py-2">
          <p class="m-0 mb-1.5 text-xs font-medium text-foreground/80">{m.companion_usage_by_kind()}</p>
          <ul class="m-0 list-none space-y-1 p-0 text-xs tabular-nums text-muted-foreground" data-testid="usage-by-kind">
            {#each summary.byKind as item (item.kind)}
              <li class="flex flex-wrap justify-between gap-2"><span class="text-foreground/85">{item.label}</span><span>{details(item.totals)}{item.totals.inputTokens ? ` · ${m.companion_usage_hit({ rate: formatHitRate(item.totals.cacheHitTokens, item.totals.inputTokens) })}` : ""} · {money(item.totals)}</span></li>
            {/each}
          </ul>
        </div>
      {/if}
      <p class="m-0 py-2 text-[11px] leading-relaxed text-muted-foreground/80">{m.companion_usage_price_source({ source: summary.priceSource })}{#if summary.ledger}{m.companion_usage_ledger({ ledger: summary.ledger })}{/if}</p>
    {/if}
    <SettingsRow label={m.companion_usage_compact_label()} description={m.companion_usage_compact_desc()}>
      {#snippet control()}
        <input type="number" min="0" step="1000" class="h-8 w-32 rounded-md border border-input bg-background px-2 text-sm tabular-nums" aria-label={m.companion_usage_compact_aria()}
          value={compactAt} disabled={savingCompact} onchange={(e) => void saveCompact(Number(e.currentTarget.value))} />
      {/snippet}
    </SettingsRow>
  </SettingsSection>
</div>
