<script lang="ts">
  // 侧边栏里的“对话”列表：新对话、切换、删除（参照 DeepSeek Harness 侧边栏的会话列表）。
  // 琴音正在回答时不能切换或删除（后端也会拒绝）。
  import { Plus, Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
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
  const sorted = $derived([...conversations].sort((a, b) => b.updatedAtMs - a.updatedAtMs));
  const busyHint = "正在回答，请先停止";

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

<section class="flex min-h-0 flex-1 flex-col" aria-label="对话列表" data-testid="agent-conversations">
  <div class="flex h-7 shrink-0 items-center justify-between pr-1 pl-2.5">
    <h2 class="text-[11px] font-medium tracking-wider text-sidebar-foreground/55">对话</h2>
    <button type="button" data-testid="agent-new-conversation" aria-label="新对话"
      disabled={running || currentId === null} title={running ? busyHint : currentId === null ? "已经是新对话" : "新对话"}
      class="flex size-6 items-center justify-center rounded-md text-sidebar-foreground/60 transition-colors hover:bg-sidebar-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      onclick={onNew}><Plus class="size-3.5" /></button>
  </div>
  <ul class="min-h-0 flex-1 space-y-px overflow-y-auto pb-1 [scrollbar-gutter:stable]">
    {#if currentId === null}
      <li><div class="flex h-8 items-center rounded-lg px-2.5 text-[13px] {activePage ? 'bg-sidebar-accent text-sidebar-accent-foreground' : 'text-sidebar-foreground'}" aria-current="true" data-testid="agent-conversation-new">新对话</div></li>
    {/if}
    {#each sorted as item (item.id)}
      {@const active = item.id === currentId}
      <li class="group/item relative">
        <button type="button" data-testid="agent-conversation" data-conversation-id={item.id} aria-current={active ? "true" : undefined}
          disabled={running && !active} title={running && !active ? busyHint : item.title}
          class="flex h-8 w-full items-center gap-2 rounded-lg pr-8 pl-2.5 text-left text-[13px] transition-colors disabled:cursor-not-allowed disabled:opacity-50
            {active && activePage ? 'bg-sidebar-accent text-sidebar-accent-foreground' : 'text-sidebar-foreground hover:bg-sidebar-accent/60 hover:text-foreground'}"
          onclick={() => onSelect(item.id)}>
          {#if item.id === runningId}<span class="size-1.5 shrink-0 animate-pulse rounded-full bg-primary" aria-label="回答中"></span>{/if}
          {#if item.parent}<span class="shrink-0 rounded bg-sidebar-accent px-1 text-[10px] text-sidebar-foreground/60" data-testid="agent-conversation-child" title="子会话（只能查看）">子</span>{/if}
          <span class="min-w-0 flex-1 truncate">{item.title || "（无标题）"}</span>
          <span class="shrink-0 text-[11px] text-sidebar-foreground/45 tabular-nums group-hover/item:invisible">{when(item.updatedAtMs)}</span>
        </button>
        <button type="button" aria-label={`删除对话：${item.title}`} data-testid="agent-conversation-delete"
          disabled={running && active} title={running && active ? busyHint : "删除"}
          class="absolute top-1/2 right-1 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-sidebar-foreground/60 opacity-0 transition-opacity group-hover/item:opacity-100 hover:bg-destructive/15 hover:text-destructive focus-visible:opacity-100 disabled:hidden"
          onclick={() => { confirm = item; }}>
          <Trash2 class="size-3.5" />
        </button>
      </li>
    {/each}
    {#if !sorted.length && currentId !== null}
      <li class="px-2.5 py-2 text-xs text-sidebar-foreground/50">还没有对话</li>
    {/if}
  </ul>
</section>

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
