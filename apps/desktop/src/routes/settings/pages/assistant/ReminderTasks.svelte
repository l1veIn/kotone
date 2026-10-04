<script lang="ts">
  import { onMount } from "svelte";
  import { isTauri } from "../../../../lib/ipc";
  import {
    addPreviewReminder, cancelAssistantRequest, cancelReminder, listReminders,
    submitAssistantText, subscribeAssistant, subscribeReminders,
    type Reminder,
  } from "../../../../lib/assistant";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { appState } from "../../../../lib/stores/state";
  import AssistantVoiceInput from "./AssistantVoiceInput.svelte";

  let { ready, active, onOpenSettings }: { ready: boolean; active: boolean; onOpenSettings: () => void } = $props();
  let voiceBusy = $state(false);
  const inputBusy = $derived(voiceBusy || !["idle", "success", "error"].includes($appState.state));
  let reminders = $state<Reminder[]>([]);
  let now = $state(Date.now());
  let loading = $state(true);
  let loadError = $state("");
  let text = $state("");
  let reply = $state("");
  let requestError = $state("");
  let submitting = $state(false);
  let cancelling = $state(false);
  let disposed = false;
  let refreshVersion = 0;
  const browserPreview = !isTauri;
  const pending = $derived(reminders.filter((item) => item.status === "pending" || item.status === "delivering")
    .sort((a, b) => a.dueAtMs - b.dueAtMs));
  const recent = $derived(reminders.filter((item) => item.status !== "pending" && item.status !== "delivering")
    .sort((a, b) => b.createdAtMs - a.createdAtMs).slice(0, 20));
  const statuses: Record<Reminder["status"], string> = {
    pending: "等待提醒", delivering: "正在提醒", delivered: "已提醒", failed: "提醒失败",
    cancelled: "已取消", missed: "已错过",
  };

  async function refresh() {
    const version = ++refreshVersion;
    try {
      const items = await listReminders();
      if (disposed || version !== refreshVersion) return;
      reminders = items;
      loadError = "";
    } catch (error) {
      if (!disposed && version === refreshVersion) loadError = errText(error);
    } finally {
      if (!disposed && version === refreshVersion) loading = false;
    }
  }

  onMount(() => {
    const cleanups: (() => void)[] = [];
    const retain = (cleanup: () => void) => disposed ? cleanup() : cleanups.push(cleanup);
    void subscribeReminders(() => { void refresh(); }).then(retain).catch((error) => {
      if (!disposed) loadError = `提醒状态订阅失败：${errText(error)}`;
    });
    void subscribeAssistant((event) => {
      if (disposed) return;
      reply = event.message || (event.reminders.length ? `已安排 ${event.reminders.length} 条提醒` : "没有新增提醒");
      void refresh();
    }).then(retain).catch((error) => {
      if (!disposed) loadError = `助手状态订阅失败：${errText(error)}`;
    });
    void refresh();
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => {
      disposed = true;
      refreshVersion++;
      clearInterval(timer);
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

  async function cancel(item: Reminder) {
    try {
      const cancelled = await cancelReminder(item.id);
      if (!cancelled) toast(false, "提醒状态已经变化，请查看最新列表");
      await refresh();
    } catch (error) { toast(false, `取消失败：${errText(error)}`); }
  }

  function countdown(due: number) {
    const seconds = Math.max(0, Math.ceil((due - now) / 1000));
    if (!seconds) return "即将提醒";
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return `${hours ? `${hours} 小时 ` : ""}${minutes ? `${minutes} 分 ` : ""}${seconds % 60} 秒后`;
  }
</script>

{#if browserPreview}
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-300/20 bg-amber-300/5 px-4 py-3 text-xs text-amber-200"><p role="status">界面预览，不创建真实提醒。</p><button type="button" class="secondary" onclick={addPreviewReminder}>添加示例提醒</button></div>
{/if}
<section class="panel" aria-labelledby="assistant-input-title">
  <div class="flex items-start justify-between gap-4">
    <h2 id="assistant-input-title" class="shrink-0 pt-2">创建提醒</h2>
    <AssistantVoiceInput {ready} {active} {submitting} {onOpenSettings} onBusyChange={(busy) => { voiceBusy = busy; }} />
  </div>
  {#if !ready}<div class="notice">请先选择 AI 连接。<button type="button" class="text-kotone-cyan hover:underline" onclick={onOpenSettings}>前往设置 →</button></div>{/if}
  <form class="mt-4" onsubmit={submit}>
    <label class="mb-2 block text-xs text-white/65" for="assistant-request">提醒内容</label>
    <textarea id="assistant-request" data-testid="assistant-request" bind:value={text} rows="3" maxlength="2000"
      placeholder="例如：5 分钟后提醒我打野有闪" disabled={submitting || inputBusy}
      class="w-full resize-y rounded-xl border border-white/12 bg-black/15 px-3 py-2.5 text-sm text-white/90 outline-none focus:border-kotone-cyan/60 disabled:opacity-60"></textarea>
    <div class="mt-2 flex items-center justify-between gap-3">
      <p class="text-[11px] text-white/40">单次提醒 · 1 秒–7 天</p>
      <div class="flex gap-2">
        {#if submitting}<button type="button" class="secondary" disabled={cancelling} onclick={cancelRequest}>{cancelling ? "取消中…" : "取消请求"}</button>{/if}
        <button type="submit" class="primary" disabled={submitting || inputBusy || !text.trim() || (!ready && !browserPreview)}>{submitting ? "创建中…" : "创建提醒"}</button>
      </div>
    </div>
  </form>
  {#if reply}<p class="notice text-kotone-cyan" role="status">{reply}</p>{/if}
  {#if requestError}<p class="notice text-kotone-pink" role="alert">{requestError}</p>{/if}
</section>

  <section class="panel" aria-labelledby="reminders-title">
    <div class="flex items-center justify-between gap-3"><h2 id="reminders-title">待办提醒 <span class="ml-1 text-kotone-cyan">{pending.length}</span></h2><button class="secondary" type="button" onclick={refresh}>刷新</button></div>
    <p class="mt-2 text-xs leading-relaxed text-white/45">停止识别不影响提醒；退出 Kotone 后不提醒。</p>
    {#if loadError}<p role="alert" class="notice text-kotone-pink">{loadError}</p>{/if}
    {#if loading}<p class="empty">正在读取提醒…</p>
    {:else if !pending.length}<p class="empty">暂无待办提醒</p>
    {:else}
      <ul class="mt-4 space-y-2">
        {#each pending as item (item.id)}
          <li class="reminder-row">
            <div class="min-w-0"><p class="break-words text-sm text-white/90">{item.text}</p><p class="mt-1 text-xs text-white/45">{item.delivery.map((kind) => kind === "voice" ? "语音" : "弹幕").join(" + ")} · {new Date(item.dueAtMs).toLocaleString()}</p></div>
            <div class="flex shrink-0 items-center gap-3"><span class="text-xs tabular-nums text-kotone-cyan">{item.status === "delivering" ? statuses[item.status] : countdown(item.dueAtMs)}</span><button type="button" class="secondary" onclick={() => cancel(item)}>取消</button></div>
          </li>
        {/each}
      </ul>
    {/if}
    {#if recent.length}
      <details class="mt-5 border-t border-white/8 pt-3"><summary class="cursor-pointer text-xs text-white/55">最近记录（{recent.length}）</summary><ul class="mt-3 space-y-2">
        {#each recent as item (item.id)}<li class="reminder-row"><div class="min-w-0"><p class="break-words text-sm text-white/65">{item.text}</p>{#if item.error}<p class="mt-1 text-xs text-kotone-pink">{item.error}</p>{/if}</div><span class="shrink-0 text-xs text-white/45">{statuses[item.status]}</span></li>{/each}
      </ul></details>
    {/if}
  </section>
