<script lang="ts">
  // 一次 BetterGI 的运行（插件页“最近的运行”的一行，也是推送卡片的正文）：跑的是什么、名字、现在怎样、几点；
  // 进行到哪、出过的错、BetterGI 最后说的话。名字和原话是 BetterGI / 玩家写的字：只当文字显示。
  import { m } from "$lib/paraglide/messages.js";
  import type { GenshinRun } from "./genshin";
  import { kindLabel, statusLabel, tone } from "./labels";

  let { run }: { run: GenshinRun } = $props();
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
</script>

<div class="min-w-0" data-testid="genshin-run" data-status={run.status}>
  <div class="flex items-baseline gap-2">
    <span class="min-w-0 flex-1 truncate text-[13px] text-foreground">
      <span class="text-muted-foreground">{kindLabel(run.kind)}</span>{#if run.name} {run.name}{/if}
    </span>
    <span class="shrink-0 text-xs font-medium tone-{tone(run.status)}" data-testid="genshin-run-status">{statusLabel(run.status)}</span>
    <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{clock(run.finishedAtMs ?? run.startedAtMs)}</span>
  </div>
  {#if run.step && (run.status === "running" || run.status === "starting")}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_genshin_card_step({ step: run.step })}</p>{/if}
  {#if run.pending?.length && run.status === "running"}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_genshin_card_pending({ names: run.pending.join(", ") })}</p>{/if}
  {#if run.message}<p class="m-0 break-words text-[11px] text-muted-foreground" data-testid="genshin-run-message">{run.message}</p>{/if}
  {#if run.error && run.error !== run.message}<p class="m-0 break-words text-[11px] tone-bad">{m.plugin_genshin_card_error({ error: run.error })}</p>{/if}
  {#if run.byKotone}<p class="m-0 text-[11px] text-muted-foreground">{m.plugin_genshin_card_by_kotone()}</p>{/if}
</div>

<style>
  .tone-live { color: light-dark(#a1621a, #f0c27a); }
  .tone-good { color: light-dark(#1d7a4a, #8fdcaf); }
  .tone-bad { color: light-dark(#b4232f, #ff9aa2); }
  .tone-plain { color: var(--muted-foreground); }
</style>
