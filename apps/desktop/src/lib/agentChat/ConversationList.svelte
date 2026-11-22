<script lang="ts">
  // 侧边栏里的“对话”列表：新对话、切换、删除（参照 DeepSeek Harness 侧边栏的会话列表）。
  // 琴音正在回答时不能切换或删除（后端也会拒绝）。
  import { Plus, Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import * as Dialog from "$lib/components/ui/dialog";
  import type { ConversationSummary } from "../agent";

  let { conversations, currentId, runningId, activePage, onNew, onSelect, onDelete }: {
    conversations: ConversationSummary[];
    currentId: string | null;
    runningId: string | null;
    /** 当前是否在「琴音」页（只有在时才高亮当前对话）。 */
    activePage: boolean;
    onNew: () => void;
    onSelect: (id: string) => void;
    onDelete: (id: string) => Promise<void>;
  } = $props();

  let confirm = $state<ConversationSummary | null>(null);
  let deleting = $state(false);
  const running = $derived(runningId !== null);
  // 后台做事的过程（ADR 029，旧版的“子会话”也一样）不出现在列表里，只在主对话的卡片里看。
  const sorted = $derived(conversations.filter((c) => !c.parent).sort((a, b) => b.updatedAtMs - a.updatedAtMs));
  const busyHint = m.chat_conversations_busy_hint();

  function when(ms: number): string {
    const date = new Date(ms);
    return date.toDateString() === new Date().toDateString()
      ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(date)
      : new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" }).format(date);
  }

  async function remove() {
    if (!confirm) return;
    deleting = true;
    try { await onDelete(confirm.id); confirm = null; }
    finally { deleting = false; }
  }
</script>

<section class="flex min-h-0 flex-1 flex-col" aria-label={m.chat_conversations_label()} data-testid="agent-conversations">
  <div class="flex h-7 shrink-0 items-center justify-between pr-1 pl-2.5">
    <h2 class="text-[11px] font-medium tracking-wider text-sidebar-foreground/55">{m.chat_conversations_heading()}</h2>
    <button type="button" data-testid="agent-new-conversation" aria-label={m.chat_conversations_new()}
      disabled={running || currentId === null} title={running ? busyHint : currentId === null ? m.chat_conversations_already_new() : m.chat_conversations_new()}
      class="flex size-6 items-center justify-center rounded-md text-sidebar-foreground/60 transition-colors hover:bg-sidebar-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      onclick={onNew}><Plus class="size-3.5" /></button>
  </div>
  <ul class="min-h-0 flex-1 space-y-px overflow-y-auto pb-1 [scrollbar-gutter:stable]">
    {#if currentId === null}
      <li><div class="flex h-8 items-center rounded-lg px-2.5 text-[13px] {activePage ? 'bg-sidebar-accent text-sidebar-accent-foreground' : 'text-sidebar-foreground'}" aria-current="true" data-testid="agent-conversation-new">{m.chat_conversations_new()}</div></li>
    {/if}
    {#each sorted as item (item.id)}
      {@const active = item.id === currentId}
      <li class="group/item relative">
        <button type="button" data-testid="agent-conversation" data-conversation-id={item.id} aria-current={active ? "true" : undefined}
          disabled={running && !active} title={running && !active ? busyHint : item.title}
          class="flex h-8 w-full items-center gap-2 rounded-lg pr-8 pl-2.5 text-left text-[13px] transition-colors disabled:cursor-not-allowed disabled:opacity-50
            {active && activePage ? 'bg-sidebar-accent text-sidebar-accent-foreground' : 'text-sidebar-foreground hover:bg-sidebar-accent/60 hover:text-foreground'}"
          onclick={() => onSelect(item.id)}>
          {#if item.id === runningId}<span class="size-1.5 shrink-0 animate-pulse rounded-full bg-primary" aria-label={m.chat_conversations_running()}></span>{/if}
          <span class="min-w-0 flex-1 truncate">{item.title || m.chat_conversations_untitled()}</span>
          <span class="shrink-0 text-[11px] text-sidebar-foreground/45 tabular-nums group-hover/item:invisible">{when(item.updatedAtMs)}</span>
        </button>
        <button type="button" aria-label={m.chat_conversations_delete_label({ title: item.title })} data-testid="agent-conversation-delete"
          disabled={running && active} title={running && active ? busyHint : m.chat_conversations_delete()}
          class="absolute top-1/2 right-1 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-sidebar-foreground/60 opacity-0 transition-opacity group-hover/item:opacity-100 hover:bg-destructive/15 hover:text-destructive focus-visible:opacity-100 disabled:hidden"
          onclick={() => { confirm = item; }}>
          <Trash2 class="size-3.5" />
        </button>
      </li>
    {/each}
    {#if !sorted.length && currentId !== null}
      <li class="px-2.5 py-2 text-xs text-sidebar-foreground/50">{m.chat_conversations_empty()}</li>
    {/if}
  </ul>
</section>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open && !deleting) confirm = null; }}>
  <Dialog.Content class="ui-scope" data-testid="agent-delete-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.chat_conversations_delete_title()}</Dialog.Title>
      <Dialog.Description>{m.chat_conversations_delete_description({ title: confirm?.title ?? "" })}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={deleting} onclick={() => { confirm = null; }}>{m.chat_conversations_cancel()}</Button>
      <Button variant="destructive" size="sm" data-testid="agent-delete-confirm" disabled={deleting} onclick={() => void remove()}>{m.chat_conversations_delete()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
