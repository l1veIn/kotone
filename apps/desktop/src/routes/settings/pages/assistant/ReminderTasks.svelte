<script lang="ts">
  import { onMount } from "svelte";
  import { isTauri } from "../../../../lib/ipc";
  import {
    addPreviewReminder, cancelAssistantRequest, cancelReminder, listReminders,
    submitAssistantText, subscribeAssistant, subscribeReminders,
    type Reminder,
  } from "../../../../lib/assistant";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
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
    pending: m.companion_reminder_status_pending(), delivering: m.companion_reminder_status_delivering(),
    delivered: m.companion_reminder_status_delivered(), failed: m.companion_reminder_status_failed(),
    cancelled: m.companion_reminder_status_cancelled(), missed: m.companion_reminder_status_missed(),
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
      if (!disposed) loadError = m.companion_reminder_subscribe_failed({ error: errText(error) });
    });
    void subscribeAssistant((event) => {
      if (disposed || !event.sessionId.startsWith("legacy-reminder-")) return;
      reply = event.message || (event.reminders.length ? m.companion_reminder_scheduled({ count: event.reminders.length }) : m.companion_reminder_none_added());
      void refresh();
    }).then(retain).catch((error) => {
      if (!disposed) loadError = m.companion_reminder_assistant_subscribe_failed({ error: errText(error) });
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
      const plan = await submitAssistantText(text.trim(), true);
      if (disposed) return;
      reply = plan.message || (plan.reminders.length ? m.companion_reminder_scheduled({ count: plan.reminders.length }) : m.companion_reminder_none_added());
      if (plan.reminders.length) text = "";
      await refresh();
    } catch (error) { if (!disposed) requestError = errText(error); }
    finally { if (!disposed) { submitting = false; cancelling = false; } }
  }

  async function cancelRequest() {
    cancelling = true;
    try { await cancelAssistantRequest(); }
    catch (error) { toast(false, m.companion_reminder_cancel_failed({ error: errText(error) })); }
    finally { cancelling = false; }
  }

  async function cancel(item: Reminder) {
    try {
      const cancelled = await cancelReminder(item.id);
      if (!cancelled) toast(false, m.companion_reminder_status_changed());
      await refresh();
    } catch (error) { toast(false, m.companion_reminder_cancel_failed({ error: errText(error) })); }
  }

  function countdown(due: number) {
    const seconds = Math.max(0, Math.ceil((due - now) / 1000));
    if (!seconds) return m.companion_reminder_due_now();
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return `${hours ? `${m.companion_reminder_hours({ hours })} ` : ""}${minutes ? `${m.companion_reminder_minutes({ minutes })} ` : ""}${m.companion_reminder_seconds_later({ seconds: seconds % 60 })}`;
  }
</script>

{#if browserPreview}
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-300/20 bg-amber-300/5 px-4 py-3 text-xs text-amber-200"><p role="status">{m.companion_reminder_preview_notice()}</p><button type="button" class="secondary" onclick={addPreviewReminder}>{m.companion_reminder_add_sample()}</button></div>
{/if}
<section class="panel" aria-labelledby="assistant-input-title">
  <div class="flex items-start justify-between gap-4">
    <h2 id="assistant-input-title" class="shrink-0 pt-2">{m.companion_reminder_create_title()}</h2>
    <AssistantVoiceInput {ready} {active} {submitting} {onOpenSettings} onBusyChange={(busy) => { voiceBusy = busy; }} />
  </div>
  {#if !ready}<div class="notice">{m.companion_reminder_choose_connection()}<button type="button" class="text-primary hover:underline" onclick={onOpenSettings}>{m.companion_reminder_open_settings()}</button></div>{/if}
  <form class="mt-4" onsubmit={submit}>
    <label class="mb-2 block text-xs text-muted-foreground" for="assistant-request">{m.companion_reminder_text_label()}</label>
    <textarea id="assistant-request" data-testid="assistant-request" bind:value={text} rows="3" maxlength="2000"
      placeholder={m.companion_reminder_text_placeholder()} disabled={submitting || inputBusy}
      class="w-full resize-y rounded-xl border border-white/12 bg-black/15 px-3 py-2.5 text-sm text-foreground outline-none focus:border-kotone-cyan/60 disabled:opacity-60"></textarea>
    <div class="mt-2 flex items-center justify-between gap-3">
      <p class="text-[11px] text-foreground/40">{m.companion_reminder_range_hint()}</p>
      <div class="flex gap-2">
        {#if submitting}<button type="button" class="secondary" disabled={cancelling} onclick={cancelRequest}>{cancelling ? m.companion_reminder_cancelling() : m.companion_reminder_cancel_request()}</button>{/if}
        <button type="submit" class="primary" disabled={submitting || inputBusy || !text.trim() || (!ready && !browserPreview)}>{submitting ? m.companion_reminder_creating() : m.companion_reminder_create()}</button>
      </div>
    </div>
  </form>
  {#if reply}<p class="notice text-primary" role="status">{reply}</p>{/if}
  {#if requestError}<p class="notice text-destructive" role="alert">{requestError}</p>{/if}
</section>

  <section class="panel" aria-labelledby="reminders-title">
    <div class="flex items-center justify-between gap-3"><h2 id="reminders-title">{m.companion_reminder_pending_title()} <span class="ml-1 text-primary">{pending.length}</span></h2><button class="secondary" type="button" onclick={refresh}>{m.companion_reminder_refresh()}</button></div>
    <p class="mt-2 text-xs leading-relaxed text-muted-foreground">{m.companion_reminder_hint()}</p>
    {#if loadError}<p role="alert" class="notice text-destructive">{loadError}</p>{/if}
    {#if loading}<p class="empty">{m.companion_reminder_loading()}</p>
    {:else if !pending.length}<p class="empty">{m.companion_reminder_empty()}</p>
    {:else}
      <ul class="mt-4 space-y-2">
        {#each pending as item (item.id)}
          <li class="reminder-row">
            <div class="min-w-0"><p class="break-words text-sm text-foreground">{item.text}</p><p class="mt-1 text-xs text-muted-foreground">{item.delivery.map((kind) => kind === "voice" ? m.companion_reminder_delivery_voice() : m.companion_reminder_delivery_danmaku()).join(" + ")} · {new Date(item.dueAtMs).toLocaleString()}</p></div>
            <div class="flex shrink-0 items-center gap-3"><span class="text-xs tabular-nums text-primary">{item.status === "delivering" ? statuses[item.status] : countdown(item.dueAtMs)}</span><button type="button" class="secondary" onclick={() => cancel(item)}>{m.companion_cancel()}</button></div>
          </li>
        {/each}
      </ul>
    {/if}
    {#if recent.length}
      <details class="mt-5 border-t border-white/8 pt-3"><summary class="cursor-pointer text-xs text-muted-foreground">{m.companion_reminder_recent({ count: recent.length })}</summary><ul class="mt-3 space-y-2">
        {#each recent as item (item.id)}<li class="reminder-row"><div class="min-w-0"><p class="break-words text-sm text-muted-foreground">{item.text}</p>{#if item.error}<p class="mt-1 text-xs text-destructive">{item.error}</p>{/if}</div><span class="shrink-0 text-xs text-muted-foreground">{statuses[item.status]}</span></li>{/each}
      </ul></details>
    {/if}
  </section>
