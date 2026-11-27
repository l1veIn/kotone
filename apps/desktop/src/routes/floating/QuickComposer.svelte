<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SendHorizontal } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { agentState } from "$lib/agent";
  import type { AgentChat } from "$lib/agentChat/chatController.svelte";
  import { isImeCompositionKey, shouldSubmitOnEnter } from "$lib/agentChat/keys";
  import CallKotoneButton from "$lib/components/CallKotoneButton.svelte";
  import { errText } from "$lib/stores/ui";

  let { active, chat, conversationId, name, onInputFocus, onActiveChange }: {
    active: boolean;
    chat: AgentChat;
    conversationId: string | null;
    name: string;
    onInputFocus: (focused: boolean) => void | Promise<void>;
    onActiveChange: (active: boolean) => void;
  } = $props();

  let editor: HTMLInputElement;
  let root: HTMLDivElement;
  let draft = $state("");
  let error = $state("");
  let sending = $state(false);
  let calling = $state(false);
  let keyboard = $state(false);
  let focusRequest = 0;
  let focusChange = Promise.resolve();
  let composing = false;
  const here = $derived(conversationId ?? chat.conversation?.id ?? null);
  const ready = $derived(active && !chat.loading && !!chat.state && !chat.loadError && !chat.state.notReadyReason
    && (chat.conversation?.id ?? null) === here);
  const hasDraft = $derived(draft.trim().length > 0);
  let previousConversation: string | null = null;
  let draftGeneration = 0;

  $effect.pre(() => {
    const next = here;
    if (previousConversation !== null && previousConversation !== next) {
      // 初次加载拿到对话 ID 时保留已输入的字；离开已知对话时结束这一份草稿。
      draftGeneration++;
      draft = "";
      error = "";
      untrack(() => { editor?.blur(); releaseFocus(); });
    }
    previousConversation = next;
  });

  $effect(() => { onActiveChange(active && (keyboard || sending || calling)); });
  $effect(() => { if (!active) releaseFocus(); });

  // 按下、失焦、收起连着来时，原生窗口也按相同顺序启用和还回焦点。
  function setInputFocus(focused: boolean): Promise<void> {
    const change = focusChange.then(() => onInputFocus(focused));
    focusChange = change.catch(() => {});
    return change;
  }

  function releaseFocus() {
    if (!keyboard) return;
    keyboard = false;
    focusRequest += 1;
    composing = false;
    void setInputFocus(false).catch(() => {});
  }

  async function requestFocus() {
    if (!active || keyboard) return;
    error = "";
    keyboard = true;
    const request = ++focusRequest;
    try {
      await setInputFocus(true);
      if (active && request === focusRequest) editor.focus({ preventScroll: true });
    } catch (failure) {
      if (request !== focusRequest) return;
      releaseFocus();
      error = errText(failure);
    }
  }

  function blur(event: FocusEvent) {
    if (event.relatedTarget instanceof Node && root.contains(event.relatedTarget)) return;
    releaseFocus();
  }

  async function send() {
    if (!ready || !hasDraft || sending) return;
    const text = draft.trim();
    const generation = draftGeneration;
    sending = true;
    error = "";
    try {
      await chat.send(text, here ?? undefined);
      if (generation === draftGeneration && draft.trim() === text) draft = "";
    } catch (failure) {
      if (generation === draftGeneration) error = errText(failure);
    } finally {
      sending = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !composing && !isImeCompositionKey(event)) {
      editor.blur();
      return;
    }
    if (!shouldSubmitOnEnter(event, composing)) return;
    event.preventDefault();
    void send();
  }

  async function beforeCall(): Promise<boolean> {
    if (!active) return false;
    error = "";
    const expected = here;
    const state = await agentState();
    if (!active) return false;
    if (expected !== here || state.current !== expected || (state.running !== null && state.running !== expected)) {
      throw new Error(m.chat_floating_conversation_changed());
    }
    return true;
  }

  onMount(() => {
    window.addEventListener("blur", releaseFocus);
    return () => {
      window.removeEventListener("blur", releaseFocus);
      releaseFocus();
      onActiveChange(false);
    };
  });
</script>

<div bind:this={root} class="quick" data-testid="floating-composer" onfocusout={blur}>
  <div class="row">
    <div class="mic">
      <CallKotoneButton size="icon" testid="floating-composer-mic" {ready} {beforeCall}
        onBusyChange={(value) => { calling = value; }}
        onError={(failure) => { error = m.chat_call_failed({ error: errText(failure) }); }} />
    </div>
    <div class="entry">
      <input bind:this={editor} bind:value={draft} type="text" inputmode="text" maxlength="4000" autocomplete="off"
        data-testid="floating-composer-input" aria-label={m.chat_composer_label({ name })}
        placeholder={m.chat_floating_send_placeholder({ name })} disabled={!active}
        onpointerdown={(event) => { if (event.button === 0) void requestFocus(); }}
        onfocus={() => void requestFocus()}
        onkeydown={keydown}
        oncompositionstart={() => { composing = true; }} oncompositionend={() => { composing = false; }} />
      {#if hasDraft}
        <button type="button" class="send" data-testid="floating-composer-send" aria-label={m.chat_composer_send()}
          title={ready ? m.chat_composer_send() : m.chat_composer_not_ready()} disabled={!ready || sending}
          onpointerdown={(event) => event.preventDefault()} onclick={() => void send()}><SendHorizontal class="size-3.5" /></button>
      {/if}
    </div>
  </div>
  {#if error}<p class="error" role="alert" title={error} data-testid="floating-composer-error">{error}</p>{/if}
</div>

<style>
  .quick { --target: 36px; border-top: 1px solid var(--floating-line); padding-top: 8px; }
  .row { display: grid; grid-template-columns: var(--target) minmax(0, 1fr); align-items: center; gap: 8px; }
  .mic :global(button) {
    width: var(--target); height: var(--target); border: 1px solid var(--floating-line);
    background: var(--floating-raised); color: var(--floating-ink);
  }
  .mic :global(button[aria-pressed="true"]) {
    border-color: var(--brand-pink); color: var(--brand-pink);
    background: color-mix(in srgb, var(--floating-raised) 84%, var(--brand-pink));
  }
  .mic :global(button:hover:not(:disabled)) { border-color: var(--floating-muted); }
  .entry {
    display: flex; align-items: center; gap: 4px; min-width: 0; min-height: var(--target); box-sizing: border-box;
    padding: 2px 4px 2px 9px; border: 1px solid var(--floating-line); border-radius: 9px;
    background: var(--floating-raised);
  }
  .entry:focus-within { border-color: var(--floating-muted); }
  input {
    flex: 1; width: 0; min-width: 0; padding: 4px 0; outline: none; border: 0;
    color: var(--floating-ink); background: transparent; font: inherit; font-size: 12px; line-height: 18px;
    user-select: text;
  }
  input::placeholder { color: var(--floating-muted); }
  .send {
    display: grid; width: 28px; height: 28px; flex: none; place-items: center; padding: 0; border: 0;
    border-radius: 6px; background: var(--primary); color: var(--primary-foreground); cursor: pointer;
  }
  .send:disabled { opacity: .45; cursor: default; }
  .send:hover:not(:disabled) { filter: brightness(1.08); }
  .error { margin: 4px 0 0; color: var(--destructive); font-size: 11px; line-height: 1.35; max-height: 3em; overflow: auto; overflow-wrap: anywhere; }
  @media (pointer: coarse) { .quick { --target: 44px; } input { font-size: 16px; } .send { width: 36px; height: 36px; } }
</style>
