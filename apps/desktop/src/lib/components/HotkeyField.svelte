<script lang="ts">
  import { onDestroy } from "svelte";
  import { getHotkeyStatus, type NamedHotkeyStatus } from "../ipc";
  import { captureHotkey } from "../hotkeyCapture";
  import { findHotkeyConflict } from "../hotkeyCombo";
  import { runtimeStore } from "../stores/runtime";
  import { errText, settingsStore, toast } from "../stores/ui";

  let {
    id, title, value, placeholder = "如 Ctrl+F9", disabled = false, compact = false, onSave,
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
      statusError = next ? "" : "无法确认热键注册状态";
      return next;
    } catch (error) {
      if (!disposed && version === refreshVersion) {
        status = null;
        statusError = `读取热键状态失败：${errText(error)}`;
      }
      return null;
    }
  }

  async function persist(key: string) {
    if (disposed || disabled || saving) return;
    const conflict = findHotkeyConflict($settingsStore, id, key);
    if (conflict) {
      toast(false, `${title}与${conflict}（${key}）冲突，请换一个`);
      return;
    }
    saving = true;
    try {
      await onSave(key);
      if (disposed) return;
      draft = key;
      const current = await refreshStatus();
      if (disposed) return;
      if (!key) toast(true, `已关闭${title}`);
      else if (current?.error) toast(false, `${title}未生效：${current.error}`);
      else if (current?.key) toast(true, `${title}已保存并生效：${current.key}`);
      else if (!current) toast(true, `${title}已保存；暂时无法确认是否生效`);
      else toast(true, `${title}已保存，启动 Kotone 后生效：${key}`);
    } catch (error) {
      if (!disposed) {
        toast(false, `保存${title}失败：${errText(error)}`);
        void refreshStatus();
      }
    } finally { saving = false; }
  }

  function saveDraft() {
    const key = draft.trim();
    if (!key) { toast(false, `${title}不能为空；如需关闭，请点击清除`); return; }
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
      else if (result.kind === "timeout") toast(false, "录入超时，请重试");
      else toast(true, "已取消录入");
    }, { signal: controller.signal });
  }

  onDestroy(() => {
    disposed = true;
    refreshVersion++;
    captureAbort?.abort();
  });
</script>

<div data-testid={`hotkey-field-${id}`}>
  <div class="mt-3 flex flex-wrap items-center gap-2">
    <input bind:value={draft} disabled={busy} data-testid={`hotkey-input-${id}`} aria-label={title}
      class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
      {placeholder} spellcheck="false" autocomplete="off"
      onkeydown={(event) => {
        if (event.key === "Enter") { event.preventDefault(); if (!busy) saveDraft(); }
        else if (event.key === "Escape") { event.preventDefault(); draft = value; }
      }} />
    <button type="button" data-testid={`hotkey-capture-${id}`} disabled={busy} onclick={startCapture}
      class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {capturing ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60' : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}">
      {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
    </button>
    {#if draft.trim() && draft.trim() !== value}
      <button type="button" data-testid={`hotkey-save-${id}`} disabled={busy} onclick={saveDraft}
        class="rounded-lg px-2.5 py-1.5 text-xs text-kotone-cyan ring-1 ring-kotone-cyan/40 hover:bg-kotone-cyan/10 disabled:opacity-50">保存</button>
    {/if}
    {#if draft || value}
      <button type="button" data-testid={`hotkey-clear-${id}`} disabled={busy} onclick={() => persist("")}
        class="rounded-lg px-2.5 py-1.5 text-xs text-white/60 ring-1 ring-white/15 transition hover:bg-white/10 active:scale-95 disabled:opacity-50">清除</button>
    {/if}
  </div>
  {#if !compact}<p class="mt-1.5 text-[10px] leading-relaxed text-white/35">可手动输入后按 Enter 保存，或点击录入键盘组合 / 鼠标侧键。</p>{/if}
  <div aria-live="polite" data-testid={`hotkey-status-${id}`}>
    {#if status?.error}
      <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{status.error}</p>
    {:else if statusError}
      <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{statusError}</p>
    {:else if status?.key}
      <p class="mt-2 text-[11px] text-white/40">当前生效：{status.key}</p>
    {:else if !value}
      <p class="mt-2 text-[11px] text-white/40">{compact ? "未设置" : "未设置（默认关闭，不会误触发）"}</p>
    {:else}
      <p class="mt-2 text-[11px] text-white/40">已保存：{value}；启动 Kotone 后生效</p>
    {/if}
  </div>
</div>
