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

  let overview = $state<WorkspaceOverview | null>(null);
  let game = $state<string | null>(null);
  let confirm = $state<string | null>(null);

  async function refresh() {
    try {
      overview = await workspaceOverview();
      if (game && !overview.games.some((g) => g.id === game)) game = null;
      if (!game && overview.games.length) game = overview.games[0].id;
    } catch (e) {
      toast(false, `读取记忆失败：${errText(e)}`);
    }
  }
  onMount(() => { void refresh(); });

  async function removeGame() {
    if (!confirm) return;
    try {
      await workspaceDeleteGame(confirm);
      toast(true, "已删除这个游戏的记忆");
      confirm = null;
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    }
  }
</script>

<div class="space-y-6" data-testid="memory-tab">
  <SettingsSection title="关于你" description="她记下的你的习惯和偏好（称呼、语言、常玩的位置……）。">
    <div class="py-3.5">
      <WorkspaceFileEditor file="USER.md" limit={overview?.files.user.limit ?? 1375} label="关于你" testid="memory-user"
        placeholder={"- 叫我小林\n- 和队友说英语"} onsaved={() => void refresh()} />
    </div>
  </SettingsSection>

  <SettingsSection title="她的笔记" description="跨游戏的长期笔记：环境、约定、学到的教训。写满了她会自己合并。">
    <div class="py-3.5">
      <WorkspaceFileEditor file="MEMORY.md" limit={overview?.files.memory.limit ?? 2200} label="笔记" testid="memory-notes" onsaved={() => void refresh()} />
    </div>
  </SettingsSection>

  <SettingsSection title="游戏记忆" description="每个游戏一份：按键、地图、你的打法。玩这个游戏时她才会读。" testid="memory-games">
    {#snippet actions()}
      <Button size="sm" variant="ghost" onclick={() => void workspaceOpenDir()}>打开文件夹</Button>
    {/snippet}
    <div class="py-3.5">
      {#if !overview?.games.length}
        <p class="m-0 text-xs text-muted-foreground" data-testid="memory-games-empty">还没有。玩游戏时跟她说“记住……”，她会记到这个游戏的记忆里。</p>
      {:else}
        <div class="mb-3 flex flex-wrap gap-1.5" role="tablist" aria-label="游戏">
          {#each overview.games as item (item.id)}
            <div class="group flex items-center rounded-md {game === item.id ? 'bg-primary/12 text-primary' : 'bg-muted/70 text-muted-foreground'}">
              <button type="button" role="tab" aria-selected={game === item.id} class="h-7 px-2.5 text-xs font-medium"
                data-testid="memory-game" data-game-id={item.id} onclick={() => { game = item.id; }}>{item.id}</button>
              <button type="button" aria-label={`删除 ${item.id} 的记忆`} class="mr-1 flex size-5 items-center justify-center rounded opacity-60 hover:bg-destructive/15 hover:text-destructive hover:opacity-100"
                data-testid="memory-game-delete" onclick={() => { confirm = item.id; }}><Trash2 class="size-3" /></button>
            </div>
          {/each}
        </div>
        {#if game}
          {#key game}
            <WorkspaceFileEditor file={`games/${game}`} limit={overview.gameLimit} label={`${game} 的记忆`} testid="memory-game-editor" onsaved={() => void refresh()} />
          {/key}
        {/if}
      {/if}
    </div>
  </SettingsSection>
</div>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open) confirm = null; }}>
  <Dialog.Content class="sm:max-w-sm">
    <Dialog.Header>
      <Dialog.Title>删除「{confirm}」的记忆？</Dialog.Title>
      <Dialog.Description>她会忘掉这个游戏里记下的东西，不能恢复。</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => { confirm = null; }}>取消</Button>
      <Button variant="destructive" onclick={() => void removeGame()} data-testid="memory-game-delete-confirm">删除</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
