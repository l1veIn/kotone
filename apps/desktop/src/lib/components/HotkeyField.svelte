<script lang="ts">
  import { onDestroy } from "svelte";
  import { getHotkeyStatus, type NamedHotkeyStatus } from "../ipc";
  import { captureHotkey } from "../hotkeyCapture";
  import { findHotkeyConflict } from "../hotkeyCombo";
  import { asrLocalStore, runtimeStore } from "../stores/runtime";
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
  /** 还没生效时的说明（ADR 028：只有本地识别有「启动」）。 */
  const pendingText = $derived($asrLocalStore ? "启动本地语音识别后生效" : "语音识别准备好后生效");

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
      else toast(true, `${title}已保存，${pendingText}：${key}`);
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
      {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
    </button>
    {#if draft.trim() && draft.trim() !== value}
      <button type="button" data-testid={`hotkey-save-${id}`} disabled={busy} onclick={saveDraft}
        class="h-8 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground transition-[filter] hover:brightness-110 disabled:opacity-50">保存</button>
    {/if}
    {#if draft || value}
      <button type="button" data-testid={`hotkey-clear-${id}`} disabled={busy} onclick={() => persist("")}
        class="h-8 rounded-lg px-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50">清除</button>
    {/if}
  </div>
  {#if !compact}<p class="mt-1.5 mb-0 text-[11px] leading-relaxed text-muted-foreground/80">可手动输入后按 Enter 保存，或点击录入键盘组合 / 鼠标侧键。</p>{/if}
  <div aria-live="polite" data-testid={`hotkey-status-${id}`}>
    {#if status?.error}
      <p class="mt-2 mb-0 text-xs leading-relaxed text-destructive">{status.error}</p>
    {:else if statusError}
      <p class="mt-2 mb-0 text-xs leading-relaxed text-destructive">{statusError}</p>
    {:else if status?.key}
      <p class="mt-2 mb-0 flex items-center gap-1.5 text-xs text-muted-foreground"><span class="size-1.5 rounded-full bg-success"></span>当前生效：{status.key}</p>
    {:else if !value}
      <p class="mt-2 mb-0 text-xs text-muted-foreground">{compact ? "未设置" : "未设置（默认关闭，不会误触发）"}</p>
    {:else}
      <p class="mt-2 mb-0 text-xs text-muted-foreground">已保存：{value}；{pendingText}</p>
    {/if}
  </div>
</div>
