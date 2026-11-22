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
  import { m } from "$lib/paraglide/messages.js";

  let skills = $state<SkillInfo[]>([]);
  let loaded = $state(false);
  let confirm = $state<string | null>(null);
  let importing = $state(false);

  async function refresh() {
    try {
      skills = (await workspaceOverview()).skills;
    } catch (e) {
      toast(false, m.companion_skills_load_failed({ error: errText(e) }));
    } finally {
      loaded = true;
    }
  }
  onMount(() => { void refresh(); });

  async function importSkill() {
    if (!isTauri) { toast(false, m.companion_skills_import_unavailable()); return; }
    const dir = await openDialog({ directory: true, multiple: false, title: m.companion_skills_pick_title() });
    if (typeof dir !== "string") return;
    importing = true;
    try {
      const info = await workspaceImportSkill(dir);
      toast(true, m.companion_skills_imported({ name: info.name }));
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
      toast(true, m.companion_skills_deleted());
      confirm = null;
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    }
  }
</script>

<div class="space-y-6" data-testid="skills-tab">
  <SettingsSection title={m.companion_skills_title()} description={m.companion_skills_desc()} flush>
    {#snippet actions()}
      <Button size="sm" variant="ghost" onclick={() => void workspaceOpenDir()}>{m.companion_open_folder()}</Button>
      <Button size="sm" disabled={importing} onclick={() => void importSkill()} data-testid="skills-import">{importing ? m.companion_skills_importing() : m.companion_skills_import()}</Button>
    {/snippet}
    {#if loaded && !skills.length}
      <p class="m-0 px-4 py-3.5 text-xs text-muted-foreground" data-testid="skills-empty">{m.companion_skills_empty()}</p>
    {/if}
    {#each skills as skill (skill.name)}
      <div class="flex items-center gap-3 px-4 py-3" data-testid="skill" data-skill-name={skill.name}>
        <div class="min-w-0 flex-1">
          <p class="m-0 truncate font-mono text-[13px] font-medium text-foreground">{skill.name}</p>
          <p class="mt-0.5 mb-0 line-clamp-2 text-xs text-muted-foreground">{skill.description}</p>
        </div>
        <button type="button" aria-label={m.companion_skills_open_folder_aria({ name: skill.name })} title={m.companion_skills_open_folder_title()}
          class="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          onclick={() => void workspaceOpenDir(skill.name)}><FolderOpen class="size-3.5" /></button>
        <button type="button" aria-label={m.companion_skills_delete_aria({ name: skill.name })} data-testid="skill-delete"
          class="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
          onclick={() => { confirm = skill.name; }}><Trash2 class="size-3.5" /></button>
      </div>
    {/each}
  </SettingsSection>
</div>

<Dialog.Root open={confirm !== null} onOpenChange={(open) => { if (!open) confirm = null; }}>
  <Dialog.Content class="sm:max-w-sm">
    <Dialog.Header>
      <Dialog.Title>{m.companion_skills_delete_title({ name: confirm ?? "" })}</Dialog.Title>
      <Dialog.Description>{m.companion_skills_delete_desc()}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => { confirm = null; }}>{m.companion_cancel()}</Button>
      <Button variant="destructive" onclick={() => void remove()} data-testid="skill-delete-confirm">{m.companion_skills_delete_confirm()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
