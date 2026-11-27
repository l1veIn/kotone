<script lang="ts">
  // 进行中的自动化（等待中 + 运行中，提醒也算；详情在「自动化」页，ADR 013 §8）：聊天页顶栏只放一个图标 + 数字，
  // 点了去「自动化」。没有进行中的什么都不显示（2026-11-27 玩家：原来那一行“没有进行中的……在自动化里查看”太啰嗦）。
  // 读不到也不在顶栏放字：错误在「自动化」页里看得到。
  import { onMount } from "svelte";
  import { Zap } from "@lucide/svelte";
  import { listRuns, subscribeAutomations } from "../../../../lib/automation";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenAutomation }: { onOpenAutomation: () => void } = $props();
  let count = $state(0);
  let disposed = false;
  let refreshVersion = 0;

  async function refresh() {
    const version = ++refreshVersion;
    try {
      const snapshot = await listRuns();
      if (disposed || version !== refreshVersion) return;
      count = snapshot.runs.filter((run) => run.status === "pending" || run.status === "running").length;
    } catch {
      if (!disposed && version === refreshVersion) count = 0;
    }
  }

  onMount(() => {
    let cleanup: (() => void) | undefined;
    void subscribeAutomations(() => { void refresh(); })
      .then((un) => { if (disposed) un(); else cleanup = un; })
      .catch(() => {});
    void refresh();
    return () => { disposed = true; refreshVersion++; cleanup?.(); };
  });
</script>

{#if count > 0}
  <!-- 和右边的右侧栏开关同高、同圆角、同样的悬停 -->
  <button type="button" data-testid="assistant-open-summary"
    aria-label={m.companion_runs_open_count({ count })} title={m.companion_runs_open_count({ count })}
    class="flex h-7 shrink-0 items-center gap-1 rounded-md px-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
    onclick={onOpenAutomation}>
    <Zap class="size-4" aria-hidden="true" />
    <span class="text-xs font-medium tabular-nums">{count}</span>
  </button>
{/if}
