<script lang="ts">
  // 「琴音 → 技能」（ADR 022）：装好的 Skill（agentskills.io 格式的技能包，可以分享给别人）。
  // 导入 = 选一个含 SKILL.md 的文件夹；分享 = 打开它的文件夹拷走。她也能自己写 Skill（每次先问你）。
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { FolderOpen, Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { isTauri } from "../../../../lib/ipc";
  import { workspaceDeleteSkill, workspaceImportSkill, workspaceOpenDir, workspaceOverview, type SkillInfo } from "../../../../lib/workspace";
  import { errText, toast } from "../../../../lib/stores/ui";

  let skills = $state<SkillInfo[]>([]);
  let loaded = $state(false);
  let confirm = $state<string | null>(null);
  let importing = $state(false);

  async function refresh() {
    try {
      skills = (await workspaceOverview()).skills;
    } catch (e) {
      toast(false, `读取技能失败：${errText(e)}`);
    } finally {
      loaded = true;
    }
  }
  onMount(() => { void refresh(); });

  async function importSkill() {
    if (!isTauri) { toast(false, "浏览器预览不能导入 Skill"); return; }
    const dir = await openDialog({ directory: true, multiple: false, title: "选择含 SKILL.md 的文件夹" });
    if (typeof dir !== "string") return;
    importing = true;
    try {
      const info = await workspaceImportSkill(dir);
      toast(true, `已导入「${info.name}」，下一段对话生效`);
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    } finally {
      importing = false;
    }
  }

  async function remove() {
    if (!confirm) return;
    try {
      await workspaceDeleteSkill(confirm);
      toast(true, "已删除");
      confirm = null;
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    }
  }
</script>

<div class="space-y-6" data-testid="skills-tab">
  <SettingsSection title="技能" description="技能包教她做某一类事（报点用语、某个游戏的出装思路……）。格式和 agentskills.io 一样，可以拷给朋友用。" flush>
    {#snippet actions()}
      <Button size="sm" variant="ghost" onclick={() => void workspaceOpenDir()}>打开文件夹</Button>
      <Button size="sm" disabled={importing} onclick={() => void importSkill()} data-testid="skills-import">{importing ? "导入中…" : "导入"}</Button>
    {/snippet}
    {#if loaded && !skills.length}
      <p class="m-0 px-4 py-3.5 text-xs text-muted-foreground" data-testid="skills-empty">还没有技能。可以导入别人分享的，或者让她把一套做法“写成技能”。</p>
    {/if}
    {#each skills as skill (skill.name)}
      <div class="flex items-center gap-3 px-4 py-3" data-testid="skill" data-skill-name={skill.name}>
        <div class="min-w-0 flex-1">
          <p class="m-0 truncate font-mono text-[13px] font-medium text-foreground">{skill.name}</p>
          <p class="mt-0.5 mb-0 line-clamp-2 text-xs text-muted-foreground">{skill.description}</p>
        </div>
        <button type="button" aria-label={`打开 ${skill.name} 的文件夹`} title="打开文件夹（分享时拷走整个文件夹）"
          class="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          onclick={() => void workspaceOpenDir(skill.name)}><FolderOpen class="size-3.5" /></button>
        <button type="button" aria-label={`删除 ${skill.name}`} data-testid="skill-delete"
          class="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
          onclick={() => { confirm = skill.name; }}><Trash2 class="size-3.5" /></button>
      </div>
    {/each}
  </SettingsSection>
</div>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open) confirm = null; }}>
  <Dialog.Content class="sm:max-w-sm">
    <Dialog.Header>
      <Dialog.Title>删除技能「{confirm}」？</Dialog.Title>
      <Dialog.Description>整个技能文件夹都会删掉，不能恢复。</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => { confirm = null; }}>取消</Button>
      <Button variant="destructive" onclick={() => void remove()} data-testid="skill-delete-confirm">删除</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
