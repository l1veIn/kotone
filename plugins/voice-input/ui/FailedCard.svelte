<script lang="ts">
  // 推送“没发出去”（ADR 043 D10）：那句话、为什么没发出去（给人看的一句话）、一个“重发”按钮。
  // 重发是你点的：经本体“发到游戏”的入口回到原来那个游戏窗口、用原来的频道发；切不回去就不发（不打进 Kotone 自己的窗口），
  // 不是替你补按键（ADR 032）。结果由推送原地更新这张卡片（已重发 / 新的原因）；按钮下面先显示工具回的那句。
  // 卡片本身点了是打开右侧栏，按钮上的点击和按键不往外传。
  import { Check } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import { clockTime, TOOL_RESEND, type FailedCardData, type ResendResult } from "./pushes";

  let { data, bridge }: PluginViewProps = $props();
  const card = $derived(data && typeof data === "object" && "id" in data ? (data as FailedCardData) : null);

  let busy = $state(false);
  /** 这次点了以后工具回的（卡片的数据原地更新以后以数据为准）。 */
  let answer = $state<ResendResult | null>(null);
  /** 工具回的是哪一版卡片（次数变了 = 推送更新了，旧的回答不再显示）。 */
  let answeredAt = $state(0);
  const shown = $derived(answer && card && answeredAt === card.attempts ? answer : null);
  const resentTo = $derived(card?.resent?.channel.name ?? (shown?.status === "sent" ? shown.channelName ?? "" : null));

  async function resend(event: Event) {
    event.stopPropagation();
    if (!card || busy) return;
    busy = true;
    answeredAt = card.attempts;
    try {
      const result = await bridge.call(TOOL_RESEND, { id: card.id, ...(card.channel ? { channel: card.channel } : {}) });
      answer = (result.content ?? { status: "refused" }) as ResendResult;
    } catch (error) {
      answer = { status: "refused", reason: errText(error) };
    } finally {
      busy = false;
    }
  }
</script>

<div class="min-w-0" data-testid="voice-failed-card" data-state={resentTo !== null ? "resent" : busy ? "busy" : "failed"}>
  {#if card}
    <div class="flex items-baseline gap-2">
      <span class="min-w-0 flex-1 break-words text-[13px] text-foreground" data-testid="voice-failed-text">{card.text}</span>
      <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{clockTime(card.atMs)}</span>
    </div>
    <p class="m-0 mt-0.5 break-words text-xs text-warning" data-testid="voice-failed-reason">{card.reason}</p>
    {#if resentTo !== null}
      <p class="m-0 mt-1.5 flex items-center gap-1 text-xs text-success" data-testid="voice-failed-resent">
        <Check class="size-3.5 shrink-0" />{m.plugin_voice_input_card_resent({ channel: resentTo })}
      </p>
    {:else}
      <div class="mt-1.5 flex flex-wrap items-center gap-2">
        <Button size="xs" variant="outline" disabled={busy} title={m.plugin_voice_input_card_resend_title()}
          data-testid="voice-failed-resend" onclick={resend} onkeydown={(e: KeyboardEvent) => e.stopPropagation()}>
          {busy ? m.plugin_voice_input_card_resending() : m.plugin_voice_input_card_resend()}
        </Button>
        {#if card.attempts > 1}
          <span class="text-[11px] text-muted-foreground" data-testid="voice-failed-attempts">{m.plugin_voice_input_card_attempts({ count: card.attempts })}</span>
        {/if}
        {#if shown && (shown.status === "refused" || shown.status === "failed" || shown.status === "disabled")}
          <span class="min-w-0 break-words text-[11px] text-muted-foreground" data-testid="voice-failed-answer">
            {m.plugin_voice_input_card_resend_refused({ reason: shown.reason ?? shown.text ?? "" })}
          </span>
        {/if}
      </div>
    {/if}
  {:else}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_voice_input_card_empty()}</p>
  {/if}
</div>
