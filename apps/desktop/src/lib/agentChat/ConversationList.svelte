<script lang="ts">
  // 对话列表：新对话、切换、删除。琴音正在回答时不能切换或删除（后端也会拒绝）。
  import { MessageSquarePlus, Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import type { ConversationSummary } from "../agent";

  let { conversations, currentId, running, onNew, onSelect, onDelete }: {
    conversations: ConversationSummary[];
    currentId: string | null;
    running: boolean;
    onNew: () => void;
    onSelect: (id: string) => void;
    onDelete: (id: string) => Promise<void>;
  } = $props();

  let confirm = $state<ConversationSummary | null>(null);
  let deleting = $state(false);
  const sorted = $derived([...conversations].sort((a, b) => b.updatedAtMs - a.updatedAtMs));
  const busyHint = "琴音正在回答，请先停止";

  function when(ms: number): string {
    const date = new Date(ms);
    const today = new Date();
    return date.toDateString() === today.toDateString()
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

<aside class="flex h-full w-52 shrink-0 flex-col border-r bg-card/30" aria-label="对话列表" data-testid="agent-conversations">
  <div class="p-2">
    <Button variant="outline" size="sm" class="w-full justify-start" data-testid="agent-new-conversation" disabled={running || currentId === null}
      title={running ? busyHint : currentId === null ? "已经是新对话" : undefined} onclick={onNew}>
      <MessageSquarePlus class="size-3.5" />新对话
    </Button>
  </div>
  <ul class="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-2">
    {#if currentId === null}
      <li><div class="rounded-md bg-accent px-2.5 py-2 text-xs text-accent-foreground" aria-current="true" data-testid="agent-conversation-new">新对话</div></li>
    {/if}
    {#each sorted as item (item.id)}
      {@const active = item.id === currentId}
      <li class="group/item relative">
        <button type="button" data-testid="agent-conversation" data-conversation-id={item.id} aria-current={active ? "true" : undefined}
          disabled={running && !active} title={running && !active ? busyHint : item.title}
          class="flex w-full flex-col gap-0.5 rounded-md px-2.5 py-2 pr-8 text-left text-xs transition-colors disabled:cursor-not-allowed disabled:opacity-50
            {active ? 'bg-accent text-accent-foreground' : 'text-foreground/80 hover:bg-muted'}"
          onclick={() => { if (!active) onSelect(item.id); }}>
          <span class="truncate font-medium">{item.title || "（无标题）"}</span>
          <span class="text-[10px] text-muted-foreground">{when(item.updatedAtMs)} · {item.messageCount} 条</span>
        </button>
        <button type="button" aria-label={`删除对话：${item.title}`} data-testid="agent-conversation-delete"
          disabled={running && active} title={running && active ? busyHint : "删除"}
          class="absolute top-1/2 right-1.5 -translate-y-1/2 rounded p-1 text-muted-foreground opacity-0 transition-opacity group-hover/item:opacity-100 hover:bg-destructive/15 hover:text-destructive focus-visible:opacity-100 disabled:hidden"
          onclick={() => { confirm = item; }}>
          <Trash2 class="size-3.5" />
        </button>
      </li>
    {/each}
  </ul>
</aside>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open && !deleting) confirm = null; }}>
  <Dialog.Content class="ui-scope" data-testid="agent-delete-dialog">
    <Dialog.Header>
      <Dialog.Title>删除这段对话？</Dialog.Title>
      <Dialog.Description>「{confirm?.title}」会从本机删除，不能恢复。</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={deleting} onclick={() => { confirm = null; }}>取消</Button>
      <Button variant="destructive" size="sm" data-testid="agent-delete-confirm" disabled={deleting} onclick={() => void remove()}>删除</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
