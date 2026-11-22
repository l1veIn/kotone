<script lang="ts">
  import { onDestroy } from "svelte";
  import { getHotkeyStatus, type NamedHotkeyStatus } from "../ipc";
  import { captureHotkey } from "../hotkeyCapture";
  import { findHotkeyConflict } from "../hotkeyCombo";
  import { asrLocalStore, runtimeStore } from "../stores/runtime";
  import { errText, settingsStore, toast } from "../stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let {
    id, title, value, placeholder = m.voice_hotkey_placeholder({ example: "Ctrl+F9" }), disabled = false, compact = false, onSave,
  }: {
    id: string;
    title: string;
    value: string;
    placeholder?: string;
    disabled?: boolean;
    compact?: boolean;
    onSave: (key: string) => Promise<void>;
  } = $props();

  let draft = $state("");
  let saving = $state(false);
  let capturing = $state(false);
  let status = $state<NamedHotkeyStatus | null>(null);
  let statusError = $state("");
  let disposed = false;
  let refreshVersion = 0;
  let captureAbort: AbortController | null = null;
  const busy = $derived(disabled || saving || capturing);
  /** 还没生效时的说明（ADR 028：只有本地识别有「启动」）。 */
  const pendingText = $derived($asrLocalStore ? m.voice_hotkey_pending_local() : m.voice_hotkey_pending_ready());

  $effect(() => { draft = value; });
  $effect(() => {
    value;
    $runtimeStore?.phase;
    void refreshStatus();
  });

  async function refreshStatus() {
    const version = ++refreshVersion;
    try {
      const next = (await getHotkeyStatus()).named.find((item) => item.id === id) ?? null;
      if (disposed || version !== refreshVersion) return null;
      status = next;
      statusError = next ? "" : m.voice_hotkey_status_unknown();
      return next;
    } catch (error) {
      if (!disposed && version === refreshVersion) {
        status = null;
        statusError = m.voice_hotkey_status_read_failed({ error: errText(error) });
      }
      return null;
    }
  }

  async function persist(key: string) {
    if (disposed || disabled || saving) return;
    const conflict = findHotkeyConflict($settingsStore, id, key);
    if (conflict) {
      toast(false, m.voice_hotkey_conflict({ title, conflict, key }));
      return;
    }
    saving = true;
    try {
      await onSave(key);
      if (disposed) return;
      draft = key;
      const current = await refreshStatus();
      if (disposed) return;
      if (!key) toast(true, m.voice_hotkey_cleared({ title }));
      else if (current?.error) toast(false, m.voice_hotkey_not_effective({ title, error: current.error }));
      else if (current?.key) toast(true, m.voice_hotkey_saved_active({ title, key: current.key }));
      else if (!current) toast(true, m.voice_hotkey_saved_unconfirmed({ title }));
      else toast(true, m.voice_hotkey_saved_pending({ title, pending: pendingText, key }));
    } catch (error) {
      if (!disposed) {
        toast(false, m.voice_hotkey_save_failed({ title, error: errText(error) }));
        void refreshStatus();
      }
    } finally { saving = false; }
  }

  function saveDraft() {
    const key = draft.trim();
    if (!key) { toast(false, m.voice_hotkey_empty({ title })); return; }
    void persist(key);
  }

  async function startCapture() {
    if (busy || disposed) return;
    capturing = true;
    const controller = new AbortController();
    captureAbort = controller;
    await captureHotkey((result) => {
      if (disposed || controller.signal.aborted) return;
      capturing = false;
      captureAbort = null;
      if (result.kind === "combo") {
        draft = result.combo;
        void persist(result.combo);
      } else if (result.kind === "error") toast(false, result.message);
      else if (result.kind === "timeout") toast(false, m.voice_hotkey_capture_timeout());
      else toast(true, m.voice_hotkey_capture_cancelled());
    }, { signal: controller.signal });
  }

  onDestroy(() => {
    disposed = true;
    refreshVersion++;
    captureAbort?.abort();
  });
</script>

<div data-testid={`hotkey-field-${id}`}>
  <div class="mt-1.5 flex flex-wrap items-center gap-2">
    <input bind:value={draft} disabled={busy} data-testid={`hotkey-input-${id}`} aria-label={title}
      class="h-8 w-44 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30 disabled:opacity-50"
      {placeholder} spellcheck="false" autocomplete="off"
      onkeydown={(event) => {
        if (event.key === "Enter") { event.preventDefault(); if (!busy) saveDraft(); }
        else if (event.key === "Escape") { event.preventDefault(); draft = value; }
      }} />
    <button type="button" data-testid={`hotkey-capture-${id}`} disabled={busy} onclick={startCapture}
      class="h-8 rounded-lg border px-3 text-xs font-medium transition-colors active:translate-y-px disabled:opacity-70 {capturing ? 'animate-pulse border-primary/60 bg-primary/15 text-primary' : 'border-hairline bg-muted/60 text-foreground hover:bg-muted'}">
      {capturing ? m.voice_hotkey_capturing() : m.voice_hotkey_capture()}
    </button>
    {#if draft.trim() && draft.trim() !== value}
      <button type="button" data-testid={`hotkey-save-${id}`} disabled={busy} onclick={saveDraft}
        class="h-8 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground transition-[filter] hover:brightness-110 disabled:opacity-50">{m.voice_action_save()}</button>
    {/if}
    {#if draft || value}
      <button type="button" data-testid={`hotkey-clear-${id}`} disabled={busy} onclick={() => persist("")}
        class="h-8 rounded-lg px-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50">{m.voice_action_clear()}</button>
    {/if}
  </div>
  {#if !compact}<p class="mt-1.5 mb-0 text-[11px] leading-relaxed text-muted-foreground/80">{m.voice_hotkey_hint()}</p>{/if}
  <div aria-live="polite" data-testid={`hotkey-status-${id}`}>
    {#if status?.error}
      <p class="mt-2 mb-0 text-xs leading-relaxed text-destructive">{status.error}</p>
    {:else if statusError}
      <p class="mt-2 mb-0 text-xs leading-relaxed text-destructive">{statusError}</p>
    {:else if status?.key}
      <p class="mt-2 mb-0 flex items-center gap-1.5 text-xs text-muted-foreground"><span class="size-1.5 rounded-full bg-success"></span>{m.voice_hotkey_current({ key: status.key })}</p>
    {:else if !value}
      <p class="mt-2 mb-0 text-xs text-muted-foreground">{compact ? m.voice_hotkey_unset() : m.voice_hotkey_unset_long()}</p>
    {:else}
      <p class="mt-2 mb-0 text-xs text-muted-foreground">{m.voice_hotkey_saved_status({ value, pending: pendingText })}</p>
    {/if}
  </div>
</div>
