<script lang="ts">
  // 「琴音 → 记忆」（ADR 022）：关于你（USER.md）、她的笔记（MEMORY.md）、每个游戏的记忆（memory/games/<id>.md）。
  // 她会自己用 memory 工具记；这里可以看、改、删。改动在下一段对话生效。
  import { onMount } from "svelte";
  import { Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import WorkspaceFileEditor from "./WorkspaceFileEditor.svelte";
  import { workspaceDeleteGame, workspaceOpenDir, workspaceOverview, type WorkspaceOverview } from "../../../../lib/workspace";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let overview = $state<WorkspaceOverview | null>(null);
  let game = $state<string | null>(null);
  let confirm = $state<string | null>(null);

  async function refresh() {
    try {
      overview = await workspaceOverview();
      if (game && !overview.games.some((g) => g.id === game)) game = null;
      if (!game && overview.games.length) game = overview.games[0].id;
    } catch (e) {
      toast(false, m.companion_memory_load_failed({ error: errText(e) }));
    }
  }
  onMount(() => { void refresh(); });

  async function removeGame() {
    if (!confirm) return;
    try {
      await workspaceDeleteGame(confirm);
      toast(true, m.companion_memory_game_deleted());
      confirm = null;
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    }
  }
</script>

<div class="space-y-6" data-testid="memory-tab">
  <SettingsSection title={m.companion_memory_user_title()} description={m.companion_memory_user_desc()}>
    <div class="py-3.5">
      <WorkspaceFileEditor file="USER.md" limit={overview?.files.user.limit ?? 1375} label={m.companion_memory_user_title()} testid="memory-user"
        placeholder={m.companion_memory_user_placeholder()} onsaved={() => void refresh()} />
    </div>
  </SettingsSection>

  <SettingsSection title={m.companion_memory_notes_title()} description={m.companion_memory_notes_desc()}>
    <div class="py-3.5">
      <WorkspaceFileEditor file="MEMORY.md" limit={overview?.files.memory.limit ?? 2200} label={m.companion_memory_notes_label()} testid="memory-notes" onsaved={() => void refresh()} />
    </div>
  </SettingsSection>

  <SettingsSection title={m.companion_memory_games_title()} description={m.companion_memory_games_desc()} testid="memory-games">
    {#snippet actions()}
      <Button size="sm" variant="ghost" onclick={() => void workspaceOpenDir()}>{m.companion_open_folder()}</Button>
    {/snippet}
    <div class="py-3.5">
      {#if !overview?.games.length}
        <p class="m-0 text-xs text-muted-foreground" data-testid="memory-games-empty">{m.companion_memory_games_empty()}</p>
      {:else}
        <div class="mb-3 flex flex-wrap gap-1.5" role="tablist" aria-label={m.companion_memory_games_aria()}>
          {#each overview.games as item (item.id)}
            <div class="group flex items-center rounded-md {game === item.id ? 'bg-primary/12 text-primary' : 'bg-muted/70 text-muted-foreground'}">
              <button type="button" role="tab" aria-selected={game === item.id} class="h-7 px-2.5 text-xs font-medium"
                data-testid="memory-game" data-game-id={item.id} onclick={() => { game = item.id; }}>{item.id}</button>
              <button type="button" aria-label={m.companion_memory_game_delete_aria({ game: item.id })} class="mr-1 flex size-5 items-center justify-center rounded opacity-60 hover:bg-destructive/15 hover:text-destructive hover:opacity-100"
                data-testid="memory-game-delete" onclick={() => { confirm = item.id; }}><Trash2 class="size-3" /></button>
            </div>
          {/each}
        </div>
        {#if game}
          {#key game}
            <WorkspaceFileEditor file={`games/${game}`} limit={overview.gameLimit} label={m.companion_memory_game_label({ game })} testid="memory-game-editor" onsaved={() => void refresh()} />
          {/key}
        {/if}
      {/if}
    </div>
  </SettingsSection>
</div>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open) confirm = null; }}>
  <Dialog.Content class="sm:max-w-sm">
    <Dialog.Header>
      <Dialog.Title>{m.companion_memory_delete_title({ game: confirm ?? "" })}</Dialog.Title>
      <Dialog.Description>{m.companion_memory_delete_desc()}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => { confirm = null; }}>{m.companion_cancel()}</Button>
      <Button variant="destructive" onclick={() => void removeGame()} data-testid="memory-game-delete-confirm">{m.companion_memory_delete_confirm()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
