<script lang="ts">
  // 左侧栏的“全部页面”（ADR 034 第六节）：每组里的页面上移 / 下移、显示 / 隐藏，点名字直接打开（藏起来的也能从这里打开）；
  // “设置”和“关于”在底部固定。侧栏的“⋯”、页面上右键的“全部页面…”、聊天里“去看看：左侧栏 → ⋯”都打开这里。
  import { ArrowDown, ArrowUp, Eye, EyeOff, Lock, RotateCcw } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import { companionName } from "$lib/stores/companion";
  import { settingsStore } from "../../lib/stores/ui";
  import { sidebarPagesOpen } from "../../lib/stores/navigation";
  import { usesLegacyVoiceInput } from "../../lib/ipc/contracts";
  import {
    DEFAULT_SIDEBAR, arrange, canMove, isDefault, movePage, normalizePrefs, setPageHidden,
  } from "../../lib/sidebarLayout";
  import { footItems, groupLabel, navItems, presentGroups, saveSidebarPrefs, type PageId } from "./sidebarItems";

  let { onOpen }: { onOpen: (id: PageId) => void } = $props();

  const prefs = $derived(normalizePrefs($settingsStore?.ui?.sidebar));
  const items = $derived(navItems($companionName, usesLegacyVoiceInput($settingsStore?.ui)));
  const groups = presentGroups();
  const pageLists = $derived(groups.map((g) => g.pages));

  function open(id: PageId) {
    sidebarPagesOpen.set(false);
    onOpen(id);
  }
</script>

<Dialog.Root open={$sidebarPagesOpen} onOpenChange={(value) => sidebarPagesOpen.set(value)}>
  <Dialog.Content class="sm:max-w-md" data-testid="sidebar-pages-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.shell_sidebar_all_pages()}</Dialog.Title>
      <Dialog.Description>{m.shell_sidebar_all_pages_desc()}</Dialog.Description>
    </Dialog.Header>
    <div class="max-h-[60vh] space-y-4 overflow-y-auto py-1">
      {#each groups as group (group.id)}
        <section>
          <p class="m-0 px-1 pb-1.5 text-[11px] font-medium text-muted-foreground">{groupLabel(group.id)}</p>
          <div class="overflow-hidden rounded-xl border border-hairline">
            {#each arrange(group.pages, prefs) as id (id)}
              {@const item = items[id]}
              {@const hidden = prefs.hidden.includes(id)}
              {#if item}
                {@const Icon = item.icon}
                <div class="flex items-center gap-1 border-b border-hairline px-2 py-1.5 last:border-b-0" data-testid="sidebar-page-row" data-page-id={id} data-hidden={hidden}>
                  <button type="button" class="flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-1.5 py-1 text-left text-[13px] hover:bg-muted/60 {hidden ? 'text-muted-foreground' : 'text-foreground'}"
                    data-testid="sidebar-page-open" onclick={() => open(item.id)}>
                    <Icon class="size-4 shrink-0 {hidden ? 'opacity-50' : 'text-primary'}" />
                    <span class="truncate">{item.label}</span>
                    {#if hidden}<span class="shrink-0 rounded bg-muted px-1.5 text-[10px] text-muted-foreground">{m.shell_sidebar_hidden_badge()}</span>{/if}
                  </button>
                  <Button size="icon" variant="ghost" class="size-7" aria-label={m.shell_sidebar_move_up({ name: item.label })} title={m.shell_sidebar_move_up({ name: item.label })}
                    data-testid="sidebar-page-up" disabled={!canMove(pageLists, prefs, id, -1)}
                    onclick={() => void saveSidebarPrefs(movePage(pageLists, prefs, id, -1))}><ArrowUp class="size-3.5" /></Button>
                  <Button size="icon" variant="ghost" class="size-7" aria-label={m.shell_sidebar_move_down({ name: item.label })} title={m.shell_sidebar_move_down({ name: item.label })}
                    data-testid="sidebar-page-down" disabled={!canMove(pageLists, prefs, id, 1)}
                    onclick={() => void saveSidebarPrefs(movePage(pageLists, prefs, id, 1))}><ArrowDown class="size-3.5" /></Button>
                  <Button size="icon" variant="ghost" class="size-7" aria-pressed={!hidden}
                    aria-label={hidden ? m.shell_sidebar_show({ name: item.label }) : m.shell_sidebar_hide({ name: item.label })}
                    title={hidden ? m.shell_sidebar_show({ name: item.label }) : m.shell_sidebar_hide({ name: item.label })}
                    data-testid="sidebar-page-toggle" onclick={() => void saveSidebarPrefs(setPageHidden(prefs, id, !hidden))}>
                    {#if hidden}<EyeOff class="size-3.5 text-muted-foreground" />{:else}<Eye class="size-3.5" />{/if}
                  </Button>
                </div>
              {/if}
            {/each}
          </div>
        </section>
      {/each}
      <section>
        <p class="m-0 px-1 pb-1.5 text-[11px] font-medium text-muted-foreground">{m.shell_sidebar_fixed_group()}</p>
        <div class="overflow-hidden rounded-xl border border-hairline">
          {#each footItems() as item (item.id)}
            {@const Icon = item.icon}
            <div class="flex items-center gap-1 border-b border-hairline px-2 py-1.5 last:border-b-0" data-testid="sidebar-page-fixed" data-page-id={item.id}>
              <button type="button" class="flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-1.5 py-1 text-left text-[13px] hover:bg-muted/60" onclick={() => open(item.id)}>
                <Icon class="size-4 shrink-0 text-primary" /><span class="truncate">{item.label}</span>
              </button>
              <span class="flex items-center gap-1 px-2 text-[11px] text-muted-foreground" title={m.shell_sidebar_fixed_hint()}><Lock class="size-3" />{m.shell_sidebar_fixed()}</span>
            </div>
          {/each}
        </div>
      </section>
    </div>
    <Dialog.Footer class="sm:justify-between">
      <Button variant="ghost" data-testid="sidebar-pages-reset" disabled={isDefault(prefs)} onclick={() => void saveSidebarPrefs({ ...DEFAULT_SIDEBAR })}>
        <RotateCcw class="size-3.5" />{m.shell_sidebar_reset()}
      </Button>
      <Button data-testid="sidebar-pages-done" onclick={() => sidebarPagesOpen.set(false)}>{m.shell_sidebar_done()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
