<script lang="ts">
  // 「跟琴音说」：文字或语音交给陪玩（设提醒、帮忙打字、回答）。进行中的提醒与自动化在「自动化」页
  // （ADR 013 §8）；这里只显示一行概况。
  import { onMount } from "svelte";
  import { isTauri } from "../../../../lib/ipc";
  import { cancelAssistantRequest, submitAssistantText, subscribeAssistant } from "../../../../lib/assistant";
  import { listRuns, subscribeAutomations, type AutomationRun } from "../../../../lib/automation";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { appState } from "../../../../lib/stores/state";
  import AssistantVoiceInput from "./AssistantVoiceInput.svelte";

  let { ready, active, onOpenSettings, onOpenAutomation }: {
    ready: boolean; active: boolean; onOpenSettings: () => void; onOpenAutomation: () => void;
  } = $props();
  let voiceBusy = $state(false);
  const inputBusy = $derived(voiceBusy || !["idle", "success", "error"].includes($appState.state));
  let open = $state<AutomationRun[]>([]);
  let loadError = $state("");
  let text = $state("");
  let reply = $state("");
  let requestError = $state("");
  let submitting = $state(false);
  let cancelling = $state(false);
  let disposed = false;
  let refreshVersion = 0;
  const browserPreview = !isTauri;
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
    const cleanups: (() => void)[] = [];
    const retain = (cleanup: () => void) => disposed ? cleanup() : cleanups.push(cleanup);
    void subscribeAutomations(() => { void refresh(); }).then(retain).catch((error) => {
      if (!disposed) loadError = `状态订阅失败：${errText(error)}`;
    });
    void subscribeAssistant((event) => {
      if (disposed) return;
      reply = event.message || (event.reminders.length ? `已安排 ${event.reminders.length} 条提醒` : "没有新增提醒");
      void refresh();
    }).then(retain).catch((error) => {
      if (!disposed) loadError = `助手状态订阅失败：${errText(error)}`;
    });
    void refresh();
    return () => {
      disposed = true;
      refreshVersion++;
      cleanups.forEach((cleanup) => cleanup());
    };
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (submitting || inputBusy || !text.trim()) return;
    submitting = true;
    requestError = "";
    reply = "";
    try {
      const plan = await submitAssistantText(text.trim());
      if (disposed) return;
      reply = plan.message || (plan.reminders.length ? `已安排 ${plan.reminders.length} 条提醒` : "没有新增提醒");
      if (plan.reminders.length) text = "";
      await refresh();
    } catch (error) { if (!disposed) requestError = errText(error); }
    finally { if (!disposed) { submitting = false; cancelling = false; } }
  }

  async function cancelRequest() {
    cancelling = true;
    try { await cancelAssistantRequest(); }
    catch (error) { toast(false, `取消失败：${errText(error)}`); }
    finally { cancelling = false; }
  }
</script>

{#if browserPreview}
  <div class="mb-4 rounded-xl border border-amber-300/20 bg-amber-300/5 px-4 py-3 text-xs text-amber-200"><p role="status">界面预览，不会真的调用 AI。</p></div>
{/if}
<section class="panel" aria-labelledby="assistant-input-title">
  <div class="flex items-start justify-between gap-4">
    <h2 id="assistant-input-title" class="shrink-0 pt-2">跟琴音说</h2>
    <AssistantVoiceInput {ready} {active} {submitting} {onOpenSettings} onBusyChange={(busy) => { voiceBusy = busy; }} />
  </div>
  {#if !ready}<div class="notice">请先选择 AI 连接。<button type="button" class="text-kotone-cyan hover:underline" onclick={onOpenSettings}>前往设置 →</button></div>{/if}
  <form class="mt-4" onsubmit={submit}>
    <label class="mb-2 block text-xs text-white/65" for="assistant-request">想让琴音做什么</label>
    <textarea id="assistant-request" data-testid="assistant-request" bind:value={text} rows="3" maxlength="2000"
      placeholder="例如：5 分钟后提醒我打野有闪" disabled={submitting || inputBusy}
      class="w-full resize-y rounded-xl border border-white/12 bg-black/15 px-3 py-2.5 text-sm text-white/90 outline-none focus:border-kotone-cyan/60 disabled:opacity-60"></textarea>
    <div class="mt-2 flex items-center justify-end gap-2">
      {#if submitting}<button type="button" class="secondary" disabled={cancelling} onclick={cancelRequest}>{cancelling ? "取消中…" : "取消请求"}</button>{/if}
      <button type="submit" class="primary" disabled={submitting || inputBusy || !text.trim() || (!ready && !browserPreview)}>{submitting ? "处理中…" : "交给琴音"}</button>
    </div>
  </form>
  {#if reply}<p class="notice text-kotone-cyan" role="status">{reply}</p>{/if}
  {#if requestError}<p class="notice text-kotone-pink" role="alert">{requestError}</p>{/if}
</section>

<section class="panel" aria-labelledby="assistant-open-title" data-testid="assistant-open-summary">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <h2 id="assistant-open-title">进行中</h2>
    <button type="button" class="text-xs text-kotone-cyan hover:underline" onclick={onOpenAutomation}>在「自动化」里查看 →</button>
  </div>
  {#if loadError}<p role="alert" class="notice text-kotone-pink">{loadError}</p>{/if}
  <p class="mt-2 text-xs text-white/55">
    {open.length ? `${reminders} 条提醒、${programs} 个自动化正在等待或运行。` : "没有进行中的提醒或自动化。"}
    停止识别不影响提醒；退出 Kotone 后不提醒。
  </p>
</section>
