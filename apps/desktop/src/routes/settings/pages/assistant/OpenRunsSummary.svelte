<script lang="ts">
  // 进行中的提醒与自动化（详情在「自动化」页，ADR 013 §8）：聊天页顶部只显示一行概况。
  import { onMount } from "svelte";
  import { listRuns, subscribeAutomations, type AutomationRun } from "../../../../lib/automation";
  import { errText } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenAutomation }: { onOpenAutomation: () => void } = $props();
  let open = $state<AutomationRun[]>([]);
  let loadError = $state("");
  let disposed = false;
  let refreshVersion = 0;
  const reminders = $derived(open.filter((run) => run.action.kind === "speak").length);
  const programs = $derived(open.length - reminders);

  async function refresh() {
    const version = ++refreshVersion;
    try {
      const snapshot = await listRuns();
      if (disposed || version !== refreshVersion) return;
      open = snapshot.runs.filter((run) => run.status === "pending" || run.status === "running");
      loadError = "";
    } catch (error) {
      if (!disposed && version === refreshVersion) loadError = errText(error);
    }
  }

  onMount(() => {
    let cleanup: (() => void) | undefined;
    void subscribeAutomations(() => { void refresh(); })
      .then((un) => { if (disposed) un(); else cleanup = un; })
      .catch((error) => { if (!disposed) loadError = m.companion_runs_subscribe_failed({ error: errText(error) }); });
    void refresh();
    return () => { disposed = true; refreshVersion++; cleanup?.(); };
  });
</script>

<div class="flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground" data-testid="assistant-open-summary">
  {#if loadError}
    <span role="alert" class="truncate text-destructive">{loadError}</span>
  {:else}
    <span class="truncate" title={m.companion_runs_title()}>
      {open.length ? m.companion_runs_summary({ reminders, programs }) : m.companion_runs_none()}
    </span>
  {/if}
  <button type="button" class="shrink-0 text-primary hover:underline" onclick={onOpenAutomation}>{m.companion_runs_open_automation()}</button>
</div>
